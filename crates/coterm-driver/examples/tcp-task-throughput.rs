//! Opt-in loopback benchmark of the production DriverTask queues, without stdio or UI.
use anyhow::{anyhow, ensure, Result};
use coterm_core::{
    driver::{DriverNotification, HostCommand},
    model::{
        lifecycle::{CreateInstance, InstanceInfo, InstanceStatus},
        operation::{DispatchMode, InvokeRequest, OperationEffect, OperationSource},
    },
    runtime::DriverTask,
};
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::{broadcast, watch},
    time::{sleep, timeout, Instant},
};

const FRAME: usize = 16384;

fn frame(sequence: u64) -> Vec<u8> {
    let mut bytes = vec![b'a' + (sequence % 26) as u8; FRAME];
    let header = format!("CTASK {sequence:012} ");
    bytes[..header.len()].copy_from_slice(header.as_bytes());
    bytes[FRAME - 1] = b'\n';
    bytes
}

#[derive(Default)]
struct Verify {
    bytes: u64,
    valid: u64,
    missing: u64,
    corrupt: u64,
    duplicate: u64,
    next: u64,
    pending: Vec<u8>,
    lagged: u64,
}
impl Verify {
    fn feed(&mut self, data: &[u8]) {
        self.bytes += data.len() as u64;
        self.pending.extend_from_slice(data);
        while let Some(end) = self.pending.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=end).collect();
            let sequence = line
                .get(6..18)
                .and_then(|digits| std::str::from_utf8(digits).ok())
                .and_then(|digits| digits.parse::<u64>().ok());
            let Some(sequence) = sequence.filter(|sequence| line == frame(*sequence)) else {
                self.corrupt += 1;
                continue;
            };
            if sequence > self.next {
                self.missing += sequence - self.next;
            }
            if sequence < self.next {
                self.duplicate += 1;
            }
            self.next = self.next.max(sequence + 1);
            self.valid += 1;
        }
        if self.pending.len() > FRAME * 2 {
            self.corrupt += 1;
            self.pending.clear();
        }
    }
    fn report(&self, expected: u64) -> Value {
        let missing = self.missing + expected.saturating_sub(self.next);
        json!({"bytes":self.bytes,"expectedBytes":expected*FRAME as u64,"validFrames":self.valid,
            "missingFrames":missing,"corruptFrames":self.corrupt,"duplicateOrReorderedFrames":self.duplicate,
            "partialBytes":self.pending.len(),"broadcastLaggedEvents":self.lagged,
            "verified":self.bytes == expected*FRAME as u64 && self.valid == expected && missing == 0
                && self.corrupt == 0 && self.duplicate == 0 && self.pending.is_empty()})
    }
}

async fn invoke(task: &DriverTask, operation: &str, args: Value) -> Result<Value> {
    let read = matches!(operation, "status" | "read_hex" | "read_lines");
    task.request(HostCommand::Invoke(InvokeRequest {
        operation_id: operation.into(),
        driver_instance_id: Some("throughput".into()),
        operation: format!("tcp.{operation}"),
        args,
        source: OperationSource::Human,
        effect: if read {
            OperationEffect::Read
        } else {
            OperationEffect::Commit
        },
        dispatch: DispatchMode::Execute,
    }))
    .await
    .map_err(|error| anyhow!(error.message))
}

async fn pace(start: Instant, bytes: u64, rate: u64) {
    if rate == 0 {
        return;
    }
    let target = start + Duration::from_secs_f64(bytes as f64 / rate as f64);
    if target > Instant::now() {
        tokio::time::sleep_until(target).await;
    }
}

async fn collect(
    mut events: broadcast::Receiver<DriverNotification>,
    mut stop: watch::Receiver<bool>,
    pause_ms: u64,
) -> Verify {
    let mut result = Verify::default();
    let mut paused = false;
    loop {
        tokio::select! {
            _ = stop.changed() => return result,
            event = events.recv() => match event {
                Ok(DriverNotification::Event { name, payload, .. }) if name == "tcp.output" => {
                    if let Some(text) = payload["text"].as_str() { result.feed(text.as_bytes()); }
                    if !paused && pause_ms > 0 { paused = true; sleep(Duration::from_millis(pause_ms)).await; }
                }
                Err(broadcast::error::RecvError::Lagged(count)) => result.lagged += count,
                Err(broadcast::error::RecvError::Closed) => return result,
                _ => {}
            }
        }
    }
}

async fn run(seconds: u64, rate: u64, pause_ms: u64) -> Result<Value> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let task = Arc::new(DriverTask::spawn(
        coterm_driver::tcp::backend::Backend::default(),
    ));
    let events = task.events.subscribe();
    let mut completions = task
        .take_completions()
        .await
        .ok_or_else(|| anyhow!("Completions unavailable"))?;
    task.request(HostCommand::CreateInstance(CreateInstance {
        instance: InstanceInfo {
            driver_id: "coterm-driver-tcp".into(),
            driver_instance_id: "throughput".into(),
            session_id: "throughput".into(),
            label: "Throughput".into(),
            status: InstanceStatus::Ready,
        },
        config: json!({"host":"127.0.0.1","port":listener.local_addr()?.port(),"mode":"stream"}),
    }))
    .await
    .map_err(|error| anyhow!(error.message))?;
    invoke(&task, "connect", json!({})).await?;
    let (socket, _) = timeout(Duration::from_secs(5), listener.accept()).await??;
    socket.set_nodelay(true)?;
    let completion = timeout(Duration::from_secs(5), completions.recv())
        .await?
        .ok_or_else(|| anyhow!("Connect completion unavailable"))?;
    let DriverNotification::OperationCompleted { result, .. } = completion else {
        return Err(anyhow!("Connection failed"));
    };
    let epoch = result["connectionId"]
        .as_str()
        .ok_or_else(|| anyhow!("Missing connection ID"))?
        .to_owned();
    let (stop, stopped) = watch::channel(false);
    let consumer = tokio::spawn(collect(events, stopped, pause_ms));
    let (mut reader, mut writer) = socket.into_split();
    let peer = tokio::spawn(async move {
        let mut verifier = Verify::default();
        let mut bytes = [0; 4096];
        loop {
            let n = reader.read(&mut bytes).await?;
            if n == 0 {
                return Ok::<_, anyhow::Error>(verifier);
            }
            verifier.feed(&bytes[..n]);
        }
    });
    let start = Instant::now();
    let deadline = start + Duration::from_secs(seconds);
    // A continuously ready writer must not monopolize the other load generator's task.
    let produce = tokio::spawn(async move {
        let mut count = 0_u64;
        while Instant::now() < deadline {
            writer.write_all(&frame(count)).await?;
            count += 1;
            pace(start, count * FRAME as u64, rate).await;
        }
        Ok::<_, anyhow::Error>((count, writer))
    });
    let send_task = task.clone();
    let transmit = tokio::spawn(async move {
        let mut count = 0_u64;
        while Instant::now() < deadline {
            let bytes = frame(count);
            let text = String::from_utf8(bytes)?;
            let result = invoke(
                &send_task,
                "write",
                json!({"connectionId":epoch,"text":text,"lineEnding":"none"}),
            )
            .await?;
            ensure!(result["bytes"] == FRAME, "Write count mismatch");
            count += 1;
            pace(start, count * FRAME as u64, rate).await;
        }
        Ok::<_, anyhow::Error>(count)
    });
    let (produced, transmitted) = tokio::join!(produce, transmit);
    let (rx_frames, mut writer) = produced??;
    let tx_frames = transmitted??;
    let active = start.elapsed().as_secs_f64();
    writer.shutdown().await?;
    let peer = timeout(Duration::from_secs(5), peer).await???;
    sleep(Duration::from_millis(250)).await;
    let status = invoke(&task, "status", json!({})).await?;
    let replay = invoke(&task, "read_hex", json!({"cursor":0})).await?;
    let lines = invoke(&task, "read_lines", json!({"lines":1000})).await?;
    let _ = stop.send(true);
    let received = consumer.await?;
    let output = json!({"path":"production DriverTask request/notification/broadcast queues; no stdio or UI",
        "loadScheduling":"independent Tokio tasks per direction",
        "offeredBytesPerSecondPerDirection":rate,"pauseConsumerMs":pause_ms,"activeSeconds":active,
        "rxBytesPerSecond":rx_frames as f64*FRAME as f64/active,"txBytesPerSecond":tx_frames as f64*FRAME as f64/active,
        "backendRxBytes":status["rxBytes"],"backendTxBytes":status["txBytes"],
        "backendCountVerified":status["rxBytes"]==rx_frames*FRAME as u64 && status["txBytes"]==tx_frames*FRAME as u64,
        "receive":received.report(rx_frames),"send":peer.report(tx_frames),"replayGap":replay["gap"],"readLinesDropped":lines["dropped"]});
    task.request(HostCommand::Shutdown)
        .await
        .map_err(|error| anyhow!(error.message))?;
    task.close().await.map_err(|error| anyhow!(error.message))?;
    Ok(output)
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let seconds = args.next().unwrap_or_else(|| "60".into()).parse::<u64>()?;
    let rates = args
        .next()
        .unwrap_or_else(|| "1048576,5242880,10485760,0".into());
    let pause = args.next().unwrap_or_else(|| "0".into()).parse::<u64>()?;
    ensure!(
        (1..=3600).contains(&seconds) && pause <= 5000,
        "Invalid duration or consumer pause"
    );
    let mut passed = true;
    for rate in rates.split(',') {
        let rate = rate.parse::<u64>()?;
        ensure!(
            rate == 0 || (65536..=1073741824).contains(&rate),
            "Invalid rate"
        );
        let result = run(seconds, rate, pause).await?;
        println!("{result}");
        passed &= result["backendCountVerified"] == true
            && result["receive"]["verified"] == true
            && result["send"]["verified"] == true;
    }
    ensure!(passed,"Incomplete event delivery; inspect benchmark JSON instead of treating backend counters as proof of complete consumption");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifier_accepts_fragmentation_and_rejects_incomplete_streams() {
        let source: Vec<u8> = (0..3).flat_map(frame).collect();
        let mut intact = Verify::default();
        for part in source.chunks(317) {
            intact.feed(part);
        }
        assert_eq!(intact.report(3)["verified"], true);

        let mut corrupt = source.clone();
        corrupt[100] ^= 1;
        let cases = [
            source[..FRAME * 2].to_vec(),
            [&source[..117], &source[118..]].concat(),
            [&source[..FRAME], &source].concat(),
            [frame(1), frame(0), frame(2)].concat(),
            corrupt,
        ];
        for bytes in cases {
            let mut verifier = Verify::default();
            for part in bytes.chunks(4096) {
                verifier.feed(part);
            }
            assert_eq!(verifier.report(3)["verified"], false);
        }
    }

    #[test]
    fn verifier_bounds_unterminated_data() {
        let mut verifier = Verify::default();
        for _ in 0..100 {
            verifier.feed(&[b'x'; 4096]);
            assert!(verifier.pending.len() <= FRAME * 2);
        }
        assert!(verifier.corrupt > 0);
    }
}
