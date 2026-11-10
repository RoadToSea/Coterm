/** Actual framework-neutral shared frontend component in its runnable Shadow DOM example. */
import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import { findBrowser, openSession, evaluate, waitForValue, rootDir } from "./support/browser-harness.mjs";

test("command/reply blocks support number selection, keyboard, live updates, copy and theme inheritance", async t => {
  const browserPath = findBrowser();
  if (!browserPath) { t.skip("No Chrome/Edge binary; command blocks were not browser-verified"); return; }
  const modulePath = path.join(rootDir, "apps/shared/dist/browser/block-list.js");
  assert.ok(fs.existsSync(modulePath), "run pnpm test:build");
  assert.ok(fs.statSync(modulePath).mtimeMs >= fs.statSync(path.join(rootDir, "apps/shared/components/block-list.ts")).mtimeMs, "shared frontend build must be current");
  const routes = new Map([
    ["/apps/shared/examples/command-blocks.html", ["text/html; charset=utf-8", path.join(rootDir, "apps/shared/examples/command-blocks.html")]],
    ["/apps/shared/dist/browser/block-list.js", ["text/javascript; charset=utf-8", modulePath]],
  ]);
  const server = http.createServer((request, response) => {
    const route = routes.get(request.url);
    if (!route) { response.writeHead(404); response.end(); return; }
    response.writeHead(200, { "Content-Type": route[0] });
    response.end(fs.readFileSync(route[1]));
  });
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  let session;
  try {
    session = await openSession({ browserPath, totalTimeoutMs: 60_000 });
    const { client } = session;
    const run = source => evaluate(client, source);
    const errors = [];
    client.on("Runtime.exceptionThrown", event => errors.push(event.exceptionDetails.exception?.description ?? event.exceptionDetails.text));
    await client.send("Page.enable");
    await client.send("Runtime.enable");
    await client.send("Emulation.setDeviceMetricsOverride", { width: 1000, height: 720, deviceScaleFactor: 1, mobile: false });
    await client.send("Page.navigate", { url: `http://127.0.0.1:${server.address().port}/apps/shared/examples/command-blocks.html` });
    await waitForValue(client, `document.querySelector('#blocks')?.shadowRoot?.querySelectorAll('.coterm-block').length===3`);
    await run(`window.blocksRoot=document.querySelector('#blocks').shadowRoot;
      window.rows=()=>[...blocksRoot.querySelectorAll('.coterm-block')];
      window.buttons=()=>[...blocksRoot.querySelectorAll('.coterm-block-gutter')];
      window.pressed=()=>buttons().map(el=>el.getAttribute('aria-pressed'));`);
    assert.deepEqual(await run(`pressed()`), ["false", "true", "false"]);
    assert.deepEqual(await run(`rows().map(row=>[row.getAttribute('role'),row.querySelector('.coterm-block-command').textContent,row.querySelector('.coterm-block-output').textContent])`), [
      ["listitem", "pwd", "/home/operator/coterm"],
      ["listitem", "uname -sr", "Linux 6.6.36-riscv64"],
      ["listitem", "systemctl status sensor-agent", "Unit sensor-agent.service could not be found."],
    ]);
    assert.ok(await run(`rows().every(row=>getComputedStyle(row).borderTopStyle==='solid' && parseFloat(getComputedStyle(row).borderTopWidth)>=1)`));
    assert.equal(await run(`getComputedStyle(rows()[0]).backgroundColor !== getComputedStyle(rows()[1]).backgroundColor`), true, "selection is visibly highlighted");

    // Clicking/copying command or reply text never toggles a block.
    await run(`rows()[0].querySelector('.coterm-block-command').click(); rows()[0].querySelector('.coterm-block-output').click()`);
    assert.deepEqual(await run(`pressed()`), ["false", "true", "false"]);
    await run(`buttons()[0].click()`);
    assert.deepEqual(await run(`pressed()`), ["true", "true", "false"]);
    assert.ok(await run(`document.querySelector('#selection').textContent.includes('2 个块')`));
    await run(`buttons()[1].click()`);
    assert.deepEqual(await run(`pressed()`), ["true", "false", "false"]);

    // Native Enter / Space activate the number; arrows move focus without selecting.
    await run(`buttons()[0].focus()`);
    const key = async (key, code, keyCode) => {
      await client.send("Input.dispatchKeyEvent", { type: "keyDown", key, code, windowsVirtualKeyCode: keyCode,
        ...(key === "Enter" ? { text: "\r" } : key === " " ? { text: " " } : {}) });
      await client.send("Input.dispatchKeyEvent", { type: "keyUp", key, code, windowsVirtualKeyCode: keyCode });
    };
    await key("ArrowDown", "ArrowDown", 40);
    assert.equal(await run(`blocksRoot.activeElement===buttons()[1]`), true);
    await key("Enter", "Enter", 13);
    assert.deepEqual(await run(`pressed()`), ["true", "true", "false"]);
    await key("End", "End", 35);
    await key(" ", "Space", 32);
    assert.deepEqual(await run(`pressed()`), ["true", "true", "true"]);
    await key("Home", "Home", 36);
    assert.equal(await run(`blocksRoot.activeElement===buttons()[0]`), true);

    const screenshot = async name => {
      if (!process.env.COTERM_BLOCKS_SCREENSHOT_DIR) return;
      fs.mkdirSync(process.env.COTERM_BLOCKS_SCREENSHOT_DIR, { recursive: true });
      const result = await client.send("Page.captureScreenshot", { format: "png" });
      fs.writeFileSync(path.join(process.env.COTERM_BLOCKS_SCREENSHOT_DIR, `${name}.png`), Buffer.from(result.data, "base64"));
    };
    await run(`buttons()[0].click(); buttons()[2].click()`);
    await screenshot("command-blocks-dark");
    const dark = await run(`getComputedStyle(rows()[1]).backgroundColor`);
    await run(`document.querySelector('#theme').click()`);
    assert.notEqual(await run(`getComputedStyle(rows()[1]).backgroundColor`), dark);
    assert.deepEqual(await run(`pressed()`), ["false", "true", "false"]);
    await screenshot("command-blocks-light");
    for (const width of [768, 320]) {
      await client.send("Emulation.setDeviceMetricsOverride", { width, height: 720, deviceScaleFactor: 1, mobile: false });
      assert.equal(await run(`document.documentElement.scrollWidth<=innerWidth && blocksRoot.querySelector('.coterm-blocks').scrollWidth<=blocksRoot.querySelector('.coterm-blocks').clientWidth`), true, `no horizontal overflow at ${width}px`);
    }
    await screenshot("command-blocks-narrow");
    await run(`window.originalRow=rows()[2]; window.originalOutput=rows()[2].querySelector('.coterm-block-output'); document.querySelector('#append').click()`);
    assert.equal(await run(`rows()[2]===originalRow && rows()[2].querySelector('.coterm-block-output')===originalOutput && originalOutput.textContent.includes('请确认')`), true);
    assert.deepEqual(await run(`pressed()`), ["false", "true", "false"], "live output preserves selection");

    // Isolated consumer: plain-text safety, keyed update semantics and lifecycle.
    await run(String.raw`(async()=>{window.shared=await import('/apps/shared/dist/browser/block-list.js');
      window.host=document.createElement('div'); host.style.cssText='height:150px;width:280px'; document.body.replaceChildren(host);
      window.changes=[]; window.data=[{id:'a',index:41,command:'echo <script>literal</script>',output:'<img src=x onerror=alert(1)>\n中文\toutput',actor:'human'},
        {id:'b',index:42,command:'long',output:'x'.repeat(400),actor:'robot',exitCode:7}];
      window.props=()=>({blocks:data,onSelectionChange:ids=>changes.push(ids)});
      window.view=shared.mountBlockList(host,props()); window.block=id=>host.querySelector('[data-block-id="'+id+'"]');
      block('a').querySelector('button').click();
      window.kept=block('a'); window.keptButton=kept.querySelector('button'); window.keptText=kept.querySelector('.coterm-block-output').firstChild;
      window.range=document.createRange(); range.setStart(keptText,0); range.setEnd(keptText,10); getSelection().removeAllRanges(); getSelection().addRange(range);
      view.update(props());})()`);
    assert.equal(await run(`host.querySelector('img,script')===null`), true, "remote output cannot inject HTML");
    assert.equal(await run(`getSelection().toString()==='<img src=x' && block('a')===kept && kept.querySelector('.coterm-block-output').firstChild===keptText`), true, "unchanged refresh preserves copy selection and DOM");
    assert.equal(await run(`keptButton.getAttribute('aria-pressed')`), "true", "omitted selected preserves local choice");
    await run(`keptButton.focus(); data=data.map(b=>b.id==='b'?{...b,output:'updated reply'}:b); view.update(props())`);
    assert.equal(await run(`document.activeElement===keptButton`), true, "unrelated reply updates preserve keyboard focus");
    await run(`data=[{...data[1],selected:true},{...data[0],selected:false}]; view.update(props())`);
    assert.deepEqual(await run(`[...host.querySelectorAll('.coterm-block')].map(row=>[row.dataset.blockId,row.dataset.selected])`), [["b", "true"], ["a", "false"]]);
    await run(`data=[data[0]]; view.update(props()); block('b').querySelector('button').click()`);
    assert.deepEqual(await run(`changes`), [["a"], []], "removed selections are pruned and callbacks use current props");
    await run(`view.update({blocks:[],emptyText:'暂无消息',ariaLabel:'命令消息'});`);
    assert.equal(await run(`host.querySelector('[role=status]').textContent`), "暂无消息");
    assert.equal(await run(`host.querySelector('[role=list]').getAttribute('aria-label')`), "命令消息");
    await run(`window.historyBlocks=Array.from({length:24},(_,i)=>({id:'history-'+i,index:i+1,command:'read snapshot',output:'bounded reply',actor:'human'}));
      view.update({blocks:historyBlocks}); window.list=host.querySelector('.coterm-blocks'); list.scrollTop=180; window.savedScroll=list.scrollTop;
      historyBlocks[23]={...historyBlocks[23],output:'updated tail reply'}; view.update({blocks:historyBlocks});`);
    assert.ok(await run(`savedScroll>0 && list.scrollTop===savedScroll`), "updating a tail reply does not jump away from the reader's scroll position");
    await run(`view.unmount(); view.update({blocks:data}); keptButton.click()`);
    assert.equal(await run(`host.childElementCount`), 0);
    assert.deepEqual(await run(`changes`), [["a"], []], "disposed controls do not invoke old callbacks");
    assert.deepEqual(errors, []);
  } finally {
    await session?.close();
    await new Promise(resolve => server.close(resolve));
  }
});
