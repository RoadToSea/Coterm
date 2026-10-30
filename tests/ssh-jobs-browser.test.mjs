/** Exercise the real SSH frontend: task layout, selection and live-event scroll anchoring. */
import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import { findBrowser, openSession, evaluate, waitForValue } from "./support/browser-harness.mjs";

test("SSH tasks collapse to one row and keep the selected task last while live events arrive", async (t) => {
  const browserPath = findBrowser();
  if (!browserPath) { t.skip("No Chrome/Edge binary; SSH task layout was not browser-verified"); return; }
  const artifact = new URL("../plugins/plugin-ssh/dist/frontend/index.js", import.meta.url);
  const built = fs.statSync(artifact).mtimeMs;
  for (const file of ["index.ts", "i18n.ts"]) {
    assert.ok(built >= fs.statSync(new URL(`../plugins/plugin-ssh/frontend/src/${file}`, import.meta.url)).mtimeMs,
      "run pnpm --filter coterm-plugin-ssh-ui build before the SSH jobs browser test");
  }
  const moduleURL = `data:text/javascript;base64,${fs.readFileSync(artifact).toString("base64")}`;
  const session = await openSession({ browserPath, totalTimeoutMs: 45_000 });
  const { client } = session;
  const run = (source) => evaluate(client, source);
  try {
    await client.send("Runtime.enable");
    await client.send("Emulation.setDeviceMetricsOverride", { width: 1000, height: 720, deviceScaleFactor: 1, mobile: false });
    await run(`(async () => {
      document.body.style.cssText = 'margin:0;width:100vw;height:100vh';
      window.locale = 'en'; window.cancelled = []; window.states = {};
      const receipt = (output) => ({operationId:'test',status:'completed',output,error:null,proposalId:null,waitExpired:false});
      const api = {
        pluginInstanceId:'ssh-one',
        i18n:{get locale() { return window.locale; }, subscribe(fn) {
          window.translate = fn; return () => { window.localeUnsubscribed = true; };
        }},
        async invoke(operation, args) {
          if (operation === 'ssh.status') return receipt({config:{host:'localhost',port:22,username:'user'},connected:true,connectionId:'conn'});
          if (operation === 'ssh.read') return receipt({data:'',cursor:'0',gap:false,closed:false,more:false,connectionId:'conn'});
          if (operation === 'ssh.resize') return receipt({});
          if (operation === 'ssh.output') return receipt({
            done:window.states[args.operationId] ?? false,exitCode:window.states[args.operationId] ? 0 : null,cancelled:false,timedOut:false,
            stdout:{text:args.stdoutCursor === '0' ? 'output for ' + args.operationId : '',cursor:'1',more:false},
            stderr:{text:'',cursor:'0',more:false},
          });
          throw new Error('Unexpected operation: ' + operation);
        },
        async subscribeEvents(fn) { window.event = fn; return () => { window.unsubscribed = true; }; },
        async cancelOperation(id) { window.cancelled.push(id); },
      };
      const module = await import(${JSON.stringify(moduleURL)});
      window.frontend = module.mount(document.body,api);
      window.root = document.body.firstElementChild.shadowRoot;
      window.list = root.querySelector('.jobs-list');
      window.toggle = root.querySelector('.jobs-toggle');
      window.items = () => [...list.querySelectorAll('.job:not([hidden])')];
      window.task = n => 'echo task ' + String(n).padStart(3,'0');
      window.select = n => [...list.querySelectorAll('.job-select')].find(b => b.title === task(n));
      window.emit = n => window.event({name:'ssh.job',payload:{operationId:'job-' + n,text:task(n)}});
    })()`);
    await waitForValue(client, `root.querySelector('.bar span').textContent === 'Connected'`);
    assert.equal(await run(`root.querySelector('.jobs').hidden`), true, "empty picker uses no space");
    await run(`(async () => { for (let n=1;n<=48;n++) await emit(n); })()`);
    assert.equal(await run(`toggle.getAttribute('aria-expanded')`), "false");
    assert.deepEqual(await run(`items().map(el => el.querySelector('.job-select').title)`),
      Array.from({ length: 48 }, (_, i) => `echo task ${String(48 - i).padStart(3, "0")}`));
    assert.equal(await run(`new Set(items().map(el => el.offsetTop)).size`), 1);
    assert.ok(await run(`root.querySelector('.jobs').getBoundingClientRect().height <= 41`));
    assert.ok(await run(`root.querySelector('.terminal').clientHeight > 600`), "history does not squeeze the terminal");
    assert.ok(await run(`list.scrollWidth > list.clientWidth`));
    assert.ok(await run(`toggle.getBoundingClientRect().right <= innerWidth`), "toggle stays visible");

    await run(`toggle.click()`);
    assert.equal(await run(`toggle.getAttribute('aria-expanded')`), "true");
    assert.ok(await run(`list.clientHeight <= 180 && list.scrollHeight > list.clientHeight`));
    assert.ok(await run(`new Set(items().map(el => el.offsetTop)).size > 1`));
    const point = await run(`(() => { const r=list.getBoundingClientRect(); return {x:r.left+r.width/2,y:r.top+30}; })()`);
    await client.send("Input.dispatchMouseEvent", { type: "mouseWheel", ...point, deltaX: 0, deltaY: 120 });
    await waitForValue(client, `list.scrollTop > 0`, { label: "expanded task list responds to wheel scrolling" });
    const scrollTop = await run(`list.scrollTop`);
    await run(`window.savedButton=select(30); savedButton.focus({preventScroll:true}); window.states['job-30']=true;`);
    await run(`emit(30)`);
    assert.equal(await run(`select(30) === savedButton && root.activeElement === savedButton`), true,
      "status refresh keeps button identity and keyboard focus");
    assert.ok(Math.abs(await run(`list.scrollTop`) - scrollTop) <= 1, "completion keeps expanded scroll position");
    await run(`window.anchor=items().find(el=>el.getBoundingClientRect().bottom>list.getBoundingClientRect().top);
      window.anchorTop=anchor.getBoundingClientRect().top;`);
    await run(`emit(49)`);
    assert.ok(Math.abs(await run(`anchor.getBoundingClientRect().top-anchorTop`)) <= 1,
      "new task preserves the expanded viewport anchor");

    // Select with a real keyboard action; the retained prefix ends at the chosen task.
    await run(`select(30).focus()`);
    await client.send("Input.dispatchKeyEvent", { type: "keyDown", key: "Enter", code: "Enter", windowsVirtualKeyCode: 13, text: "\r" });
    await client.send("Input.dispatchKeyEvent", { type: "keyUp", key: "Enter", code: "Enter", windowsVirtualKeyCode: 13 });
    assert.equal(await run(`toggle.getAttribute('aria-expanded')`), "false");
    assert.equal(await run(`items().length`), 20);
    assert.equal(await run(`items().at(-1).querySelector('.job-select').title`), "echo task 030");
    assert.equal(await run(`new Set(items().map(el=>el.offsetTop)).size`), 1);
    assert.ok(await run(`Math.abs(list.scrollWidth-list.clientWidth-list.scrollLeft) <= 1`));
    assert.ok(await run(`select(30).getBoundingClientRect().left >= list.getBoundingClientRect().left`));
    assert.equal(await run(`root.querySelector('.job-view').hidden`), false);
    assert.match(await run(`root.querySelector('.job-body').textContent`), /output for job-30/);
    assert.equal(await run(`root.querySelector('.job-head button').disabled`), true, "completed task cannot be stopped");
    await run(`emit(50)`);
    assert.equal(await run(`items().at(-1).querySelector('.job-select').title`), "echo task 030");
    assert.ok(await run(`Math.abs(list.scrollWidth-list.clientWidth-list.scrollLeft) <= 1`),
      "new tasks do not displace the selected task");
    await run(`select(30).click()`);
    assert.equal(await run(`root.querySelector('.job-view').hidden`), false, "selecting again keeps the task open");

    await run(`toggle.click()`);
    assert.equal(await run(`items().length`), 50, "expansion restores tasks older than the selection");
    await run(`select(1).click()`);
    assert.equal(await run(`items().at(-1).querySelector('.job-select').title`), "echo task 001");
    await run(`root.querySelector('.job-head button').click()`);
    assert.deepEqual(await run(`window.cancelled`), ["job-1"]);
    await run(`window.locale='zh-CN'; window.translate()`);
    assert.equal(await run(`toggle.getAttribute('aria-label')`), "展开任务列表");
    assert.equal(await run(`list.getAttribute('aria-label')`), "后台任务");
    assert.ok(await run(`Math.abs(list.scrollWidth-list.clientWidth-list.scrollLeft) <= 1`));

    await client.send("Emulation.setDeviceMetricsOverride", { width: 320, height: 480, deviceScaleFactor: 1, mobile: false });
    await waitForValue(client, `root.querySelector('.jobs').clientWidth <= 320 && Math.abs(list.scrollWidth-list.clientWidth-list.scrollLeft) <= 1`);
    assert.ok(await run(`select(1).getBoundingClientRect().left >= list.getBoundingClientRect().left`),
      "selected task stays fully visible after resize");
    assert.ok(await run(`root.querySelector('.layout').scrollWidth <= innerWidth`));
    await run(`toggle.click()`);
    assert.ok(await run(`list.clientHeight <= 144 && list.scrollHeight > list.clientHeight`));
    await run(`toggle.click(); select(1).parentElement.querySelector('.dismiss').click()`);
    assert.equal(await run(`root.querySelector('.job-view').hidden`), true, "removing selected task closes its output");
    assert.equal(await run(`list.scrollLeft`), 0);
    assert.equal(await run(`items()[0].querySelector('.job-select').title`), "echo task 050");
    await run(`list.dispatchEvent(new WheelEvent('wheel',{deltaY:100,bubbles:true,cancelable:true}))`);
    assert.ok(await run(`list.scrollLeft > 0`), "collapsed row supports wheel navigation");
    await run(`while (list.firstElementChild) list.firstElementChild.querySelector('.dismiss').click()`);
    assert.equal(await run(`root.querySelector('.jobs').hidden`), true);
    await run(`emit(51)`);
    assert.equal(await run(`items().length`), 1);
    assert.equal(await run(`toggle.getAttribute('aria-expanded')`), "false");
    await run(`window.frontend.unmount()`);
    assert.equal(await run(`window.unsubscribed && window.localeUnsubscribed && document.body.childElementCount === 0`), true);
  } catch (error) {
    t.diagnostic(JSON.stringify(await run(`({expanded:toggle?.getAttribute('aria-expanded'),count:items?.().length,
      width:list?.clientWidth,height:list?.clientHeight,scrollWidth:list?.scrollWidth,scrollLeft:list?.scrollLeft,
      scrollTop:list?.scrollTop,notice:root?.querySelector('.notice')?.textContent})`).catch(() => null)));
    throw error;
  } finally { await session.close(); }
});
