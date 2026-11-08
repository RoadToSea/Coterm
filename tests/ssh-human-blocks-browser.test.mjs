import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { findBrowser, openSession, evaluate, waitForValue } from "./support/browser-harness.mjs";

test("echoed human terminal commands create selectable blocks", async t => {
  const browserPath = findBrowser();
  if (!browserPath) { t.skip("No browser available"); return; }
  const artifact = new URL("../plugins/plugin-ssh/dist/frontend/index.js", import.meta.url);
  for (const file of ["index.ts", "human-commands.ts", "observed-boundaries.ts"]) {
    assert.ok(fs.statSync(artifact).mtimeMs >= fs.statSync(new URL("../plugins/plugin-ssh/frontend/src/" + file, import.meta.url)).mtimeMs, "build SSH first");
  }
  const moduleURL = "data:text/javascript;base64," + fs.readFileSync(artifact).toString("base64");
  const session = await openSession({ browserPath, totalTimeoutMs: 30000 });
  const { client } = session;
  const run = source => evaluate(client, source);
  try {
    await run(`(async () => {
      document.body.style.cssText = 'margin:0;height:100vh;width:100vw';
      window.calls = []; window.frames = []; window.blocks = []; window.selected = [];
      window.sequence = 0; window.observed = null; window.version = 0; window.prompt = 'operator@host:~$ ';
      window.emit = (text, annotation = null) => {
        if (observed && text) { observed.output += text; version++; }
        const frame = { data:btoa(text), annotation, cursor:String(++sequence), connectionId:'human-connection' };
        frames.push(frame); window.event?.({ name:'ssh.terminal.output', payload:frame });
      };
      emit('operator@host:~$ ');
      const receipt = output => ({ operationId:'fixture', status:'completed', output, error:null });
      const api = {
        pluginInstanceId:'ssh-human',
        async invoke(operation, args) {
          calls.push({ operation, args });
          if (operation === 'ssh.status') return receipt({ config:{host:'host',port:22,username:'operator'}, connected:true, connectionId:'human-connection' });
          if (operation === 'ssh.resize') return receipt({});
          if (operation === 'ssh.read') {
            const frame = frames.find(f => BigInt(f.cursor) > BigInt(args.cursor));
            return receipt(frame ? {...frame, more:frame !== frames.at(-1), gap:false, closed:false} : {data:'',cursor:String(sequence),connectionId:'human-connection',more:false,gap:false,closed:false});
          }
          if (operation === 'ssh.blocks') {
            const snapshot = structuredClone({epoch:'epoch',revision:String(version),dropped:0,blocks:[...blocks].reverse(),more:false,nextBefore:null});
            if (window.holdBlocks) await new Promise(resolve => window.releaseBlocks = () => { window.holdBlocks=false; resolve(); });
            return receipt(snapshot);
          }
          if (operation === 'ssh.select_blocks') { window.selected = args.ids; return receipt({selectedCount:selected.length}); }
          if (operation === 'ssh.select_lines') return receipt({});
          if (operation === 'ssh.input') {
            const text = atob(args.data);
            if (observed) {
              const previous = observed; window.observed = null; previous.state = 'unknown'; version++;
              emit('', {kind:'end',id:previous.id,index:previous.index,actor:'human',boundary:'observed'});
            }
            if (args.observation) {
              const b = { id:'human-'+(blocks.length+1),index:blocks.length+1,command:args.observation.text,actor:'human',channel:'pty',state:'running',boundary:'observed',exitCode:null,output:'',stderr:'',commandTruncated:false,outputTruncated:false };
              blocks.push(b); window.observed = b; version++;
              emit('', {kind:'begin',id:b.id,index:b.index,actor:'human',boundary:'observed',lineOffset:args.observation.lineOffset});
            }
            const echo = () => {
              if (window.noEcho) { if (text==='\\r') emit('\\r\\noperator@host:~$ '); return; }
              if (text === '\\x1b[A') emit('\\r\\x1b[2Koperator@host:~$ echo recalled');
              else if (text === '\\x7f') emit('\\b \\b');
              else if (text === '\\r') {
                if (args.observation?.text === 'cd /') { window.prompt = 'operator@host:/$ '; emit('\\r\\n' + window.prompt); }
                else { emit('\\r\\nfile.txt\\r\\n'); emit(window.prompt.slice(0,5)); emit(window.prompt.slice(5)); }
              }
              else emit(text.replaceAll('\\x1b[200~','').replaceAll('\\x1b[201~',''));
            };
            if (window.echoDelay && text !== '\\r') setTimeout(echo, window.echoDelay); else echo();
            return receipt({submitted:true});
          }
          throw new Error('Unexpected operation ' + operation);
        },
        async subscribeEvents(fn) { window.event = fn; return () => {}; }
      };
      const module = await import(${JSON.stringify(moduleURL)});
      window.mount = () => {
        window.frontend = module.mount(document.body, api);
        window.root = document.body.firstElementChild.shadowRoot;
        window.gutters = () => [...root.querySelectorAll('.coterm-terminal-number')];
      };
      window.mount();
    })()`);
    await waitForValue(client, "root.querySelector('.bar span').textContent==='Connected'");
    await run("window.holdBlocks=true;event({name:'ssh.blocks.changed',payload:{}})");
    await waitForValue(client, "typeof releaseBlocks==='function'");
    await run("root.querySelector('.xterm-helper-textarea').focus()");
    await client.send("Input.insertText", { text:"ls" });
    await waitForValue(client, "calls.some(c=>c.operation==='ssh.input' && atob(c.args.data)==='ls')");
    // Let xterm parse the server's echo before pressing Enter.
    await run("new Promise(resolve=>setTimeout(resolve,100))");
    await client.send("Input.dispatchKeyEvent", { type:"keyDown", key:"Enter", code:"Enter", windowsVirtualKeyCode:13 });
    await waitForValue(client, "calls.some(c=>c.operation==='ssh.input' && atob(c.args.data)==='\\r')");
    await run("new Promise(resolve=>setTimeout(resolve,250))");
    assert.equal(await run("gutters().length"), 1, "manual Enter must create a gutter block without ssh.command or API annotation");
    await run("window.releaseBlocks();window.releaseBlocks=null");
    await waitForValue(client, "gutters()[0].getAttribute('aria-label').includes('Human · ls')");
    await run("new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)))");
    const firstRange = await run(`(() => {
      const screen=root.querySelector('.xterm-screen').getBoundingClientRect();
      const frame=root.querySelector('.coterm-terminal-frame').getBoundingClientRect();
      return {height:frame.height,lineHeight:root.querySelector('.xterm-rows > div').getBoundingClientRect().height,screenRight:screen.right,right:frame.right};
    })()`);
    // The fixture writes one command row, one reply row, then a fresh prompt.
    assert.ok(Math.abs(firstRange.height - 2 * firstRange.lineHeight) <= 1, "the command frame must end before the next prompt row");
    assert.ok(firstRange.right <= firstRange.screenRight + 0.5, "frame borders fit inside the terminal grid");
    assert.equal(await run("root.querySelectorAll('.coterm-command-start').length"), 0, "a block has one frame, no duplicate xterm border decoration");
    await run("gutters()[0].click()");
    await waitForValue(client, "selected.length===1");
    assert.deepEqual(await run("blocks.filter(b=>selected.includes(b.id)).map(b=>[b.command,b.boundary,b.exitCode,b.output.includes('file.txt')])"), [["ls", "observed", null, true]]);
    assert.equal(await run("calls.some(c=>c.operation==='ssh.command')"), false);

    // Readline history and editing use the rendered line, not a raw-keystroke log.
    const key = async (name, code, keyCode) => {
      await client.send("Input.dispatchKeyEvent", {type:"keyDown",key:name,code,windowsVirtualKeyCode:keyCode});
      await client.send("Input.dispatchKeyEvent", {type:"keyUp",key:name,code,windowsVirtualKeyCode:keyCode});
    };
    await run("root.querySelector('.xterm-helper-textarea').focus()");
    await key("ArrowUp", "ArrowUp", 38);
    await run("new Promise(resolve=>setTimeout(resolve,100))");
    await key("Enter", "Enter", 13);
    await waitForValue(client, "gutters().length===2");
    assert.equal(await run("blocks[1].command"), "echo recalled");
    await client.send("Input.insertText", {text:"echo ax"});
    await run("new Promise(resolve=>setTimeout(resolve,50))");
    await key("Backspace", "Backspace", 8);
    await key("Enter", "Enter", 13);
    await waitForValue(client, "gutters().length===3");
    assert.equal(await run("blocks[2].command"), "echo a");

    // Fast Enter waits for a delayed echo; the typed characters are not recorded raw.
    await run("window.echoDelay=80");
    await client.send("Input.insertText", {text:"echo fast"});
    await key("Enter", "Enter", 13);
    await waitForValue(client, "gutters().length===4");
    assert.equal(await run("blocks[3].command"), "echo fast");
    await run("window.echoDelay=0");

    // Wrapped commands anchor the number to their first row.
    const longCommand = "echo " + "x".repeat(240);
    await client.send("Input.insertText", {text:longCommand});
    await run("new Promise(resolve=>setTimeout(resolve,100))");
    await key("Enter", "Enter", 13);
    await waitForValue(client, "gutters().length===5");
    assert.equal(await run("blocks[4].command"), longCommand);
    assert.ok(await run("calls.filter(c=>c.args.observation).at(-1).args.observation.lineOffset<0"));

    // Blank Enter, password input and alternate-screen application input create no blocks.
    await key("Enter", "Enter", 13);
    await run("window.noEcho=true;emit('\\r\\nPassword: ')");
    await run("new Promise(resolve=>setTimeout(resolve,100))");
    await client.send("Input.insertText", {text:"secret-not-context"});
    await key("Enter", "Enter", 13);
    await run("window.noEcho=false;emit('\\x1b[?1049happ$ ')");
    await run("new Promise(resolve=>setTimeout(resolve,100))");
    await client.send("Input.insertText", {text:"application-input"});
    await key("Enter", "Enter", 13);
    await run("emit('\\x1b[?1049l')");
    await run("new Promise(resolve=>setTimeout(resolve,200))");
    assert.equal(await run("blocks.length"), 5);
    assert.equal(await run("JSON.stringify(blocks).includes('secret-not-context')"), false);

    // A single-line bracketed paste still forms a command; multiline paste is not
    // guessed into multiple shell executions.
    await run("emit('\\x1b[?2004h')");
    await run("new Promise(resolve=>setTimeout(resolve,50))");
    const paste = async text => run(`(() => {
      const clipboardData = new DataTransfer(); clipboardData.setData('text/plain', ${JSON.stringify(text)});
      root.querySelector('.xterm-helper-textarea').dispatchEvent(new ClipboardEvent('paste', {clipboardData,bubbles:true,cancelable:true}));
    })()`);
    await paste("echo pasted");
    await key("Enter", "Enter", 13);
    await waitForValue(client, "gutters().length===6");
    assert.equal(await run("blocks[5].command"), "echo pasted");
    await paste("echo first\necho second");
    await key("Enter", "Enter", 13);
    await run("new Promise(resolve=>setTimeout(resolve,200))");
    assert.equal(await run("blocks.length"), 6);

    // A changed-directory prompt still closes the visual block; cd has no reply.
    await client.send("Input.insertText", {text:"cd /"});
    await key("Enter", "Enter", 13);
    await waitForValue(client, "gutters().length===7");
    await waitForValue(client, `(() => {
      const frame = root.querySelector('.coterm-terminal-frame[data-block-id="human-7"]').getBoundingClientRect();
      return Math.abs(frame.height - root.querySelector('.xterm-rows > div').getBoundingClientRect().height) <= 1;
    })()`);

    // All rectangles use the same grid edge and never overlap the following block,
    // including after terminal font/column changes and soft-wrap reflow.
    for (const [width, size] of [[1100,13], [768,20], [320,16]]) {
      await client.send("Emulation.setDeviceMetricsOverride", {width,height:760,deviceScaleFactor:1,mobile:false});
      await run(`document.documentElement.style.setProperty('--coterm-terminal-font-size', '${size}px')`);
      await waitForValue(client, `(() => {
        const screen = root.querySelector('.xterm-screen').getBoundingClientRect();
        const frames = [...root.querySelectorAll('.coterm-terminal-frame:not([hidden])')].map(el=>el.getBoundingClientRect());
        return frames.length && frames.every((r,i)=>Math.abs(r.right-screen.right)<=0.5 && (i===0 || r.top>=frames[i-1].bottom-0.5));
      })()`, {label:"one aligned, non-overlapping frame per command after reflow"});
    }
    if (process.env.COTERM_SSH_BLOCKS_SCREENSHOT_DIR) {
      const directory = process.env.COTERM_SSH_BLOCKS_SCREENSHOT_DIR;
      fs.mkdirSync(directory, {recursive:true});
      const shot = await client.send("Page.captureScreenshot", {format:"png"});
      fs.writeFileSync(path.join(directory,"ssh-human-boundaries.png"),Buffer.from(shot.data,"base64"));
    }

    // Backend annotations recreate the same numbers on a frontend remount.
    await run("window.frontend.unmount();window.mount()");
    await waitForValue(client, "gutters().length===7 && gutters().every(b=>b.textContent)");
    assert.deepEqual(await run("gutters().map(b=>b.textContent)"), ["1", "2", "3", "4", "5", "6", "7"]);
    await run("gutters()[4].click()");
    await waitForValue(client, "selected.includes('human-5')");
    await run("window.frontend.unmount()");
  } finally { await session.close(); }
});
