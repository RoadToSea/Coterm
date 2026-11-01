import test from "node:test";
import assert from "node:assert/strict";
import { DEFAULT_PREFERENCES, PREFERENCES_STORAGE_KEY, resolvePreferences, getPreferences, setPreferences, subscribePreferences } from "../apps/desktop/src/preferences.ts";
import { attachTerminalPreferences, readTerminalPreferences } from "../packages/plugin-sdk/src/components/terminal-theme.ts";

test("display preferences validate saved data and retain safe defaults", () => {
  for (const value of [null, undefined, [], true, "bad"]) assert.deepEqual(resolvePreferences(value), DEFAULT_PREFERENCES);
  assert.deepEqual(resolvePreferences({interfaceFontSize:18,terminalFontSize:20,terminalLineHeight:1.4,cursorBlink:false}),
    {interfaceFontSize:18,terminalFontSize:20,terminalLineHeight:1.4,cursorBlink:false});
  for (const value of [-1, 0, 50, NaN, Infinity, "16", 13.5]) {
    assert.equal(resolvePreferences({interfaceFontSize:value}).interfaceFontSize, 14);
    assert.equal(resolvePreferences({terminalFontSize:value}).terminalFontSize, 13);
  }
  for (const value of [-1, 0, 3, Infinity, "1.2"]) assert.equal(resolvePreferences({terminalLineHeight:value}).terminalLineHeight, 1);
  assert.equal(resolvePreferences({cursorBlink:"false"}).cursorBlink, true);
  assert.equal(resolvePreferences({interfaceFontSize:20,terminalFontSize:28,terminalLineHeight:2}).terminalFontSize, 28);
});

test("display preferences persist only validated UI data, tolerate disabled storage, and release subscribers", () => {
  const original = Object.getOwnPropertyDescriptor(globalThis, "localStorage");
  const values = new Map();
  let updates = 0;
  setPreferences({...DEFAULT_PREFERENCES});
  const stop = subscribePreferences(() => updates++);
  try {
    Object.defineProperty(globalThis, "localStorage", {configurable:true,value:{setItem:(key,value)=>values.set(key,value)}});
    setPreferences({terminalFontSize:18,secret:"not a display preference"});
    assert.equal(getPreferences().terminalFontSize, 18);
    assert.deepEqual(JSON.parse(values.get(PREFERENCES_STORAGE_KEY)), {...DEFAULT_PREFERENCES,terminalFontSize:18});
    setPreferences({terminalFontSize:18});
    assert.equal(updates, 1);
    Object.defineProperty(globalThis, "localStorage", {configurable:true,get(){throw Error("disabled");}});
    setPreferences({interfaceFontSize:16,cursorBlink:false});
    assert.equal(getPreferences().interfaceFontSize, 16);
    assert.equal(getPreferences().cursorBlink, false);
    assert.equal(updates, 2);
    stop();
    setPreferences({...DEFAULT_PREFERENCES});
    assert.equal(updates, 2);
  } finally {
    stop();
    if (original) Object.defineProperty(globalThis, "localStorage", original);
    else delete globalThis.localStorage;
    setPreferences({...DEFAULT_PREFERENCES});
  }
});

test("terminal preference helpers read inherited tokens, bound values, deduplicate and disconnect", () => {
  const original=globalThis.MutationObserver;
  const tokens=new Map();
  const element={ownerDocument:{documentElement:{},defaultView:{getComputedStyle:()=>({getPropertyValue:key=>tokens.get(key)??""})}}};
  let update;
  let disconnected=false;
  globalThis.MutationObserver=class {
    constructor(callback){ update=callback; }
    observe(root,options){ assert.equal(root,element.ownerDocument.documentElement);assert.deepEqual(options.attributeFilter,["style"]); }
    disconnect(){ disconnected=true; }
  };
  try {
    assert.deepEqual(readTerminalPreferences(element),{fontSize:13,lineHeight:1,cursorBlink:true});
    const results=[];
    const stop=attachTerminalPreferences(element,next=>results.push(next));
    update(); assert.equal(results.length,0);
    tokens.set('--coterm-terminal-font-size','22px'); tokens.set('--coterm-terminal-line-height','1.4'); tokens.set('--coterm-terminal-cursor-blink','false');
    update(); update();
    assert.deepEqual(results,[{fontSize:22,lineHeight:1.4,cursorBlink:false}]);
    tokens.set('--coterm-terminal-font-size','900px'); tokens.set('--coterm-terminal-line-height','0.2');
    assert.deepEqual(readTerminalPreferences(element),{fontSize:13,lineHeight:1,cursorBlink:false});
    stop(); assert.equal(disconnected,true);
  } finally {
    if(original)globalThis.MutationObserver=original;
    else delete globalThis.MutationObserver;
  }
});
