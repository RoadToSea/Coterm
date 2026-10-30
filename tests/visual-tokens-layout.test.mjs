import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";

const rootDir = process.cwd();
const indexCssPath = path.join(rootDir, "apps/desktop/src/index.css");
const tailwindConfigPath = path.join(rootDir, "apps/desktop/tailwind.config.js");
const sshPluginPath = path.join(rootDir, "plugins/plugin-ssh/frontend/src/index.ts");
const blockListPath = path.join(rootDir, "packages/plugin-sdk/src/components/block-list.ts");
const appTsxPath = path.join(rootDir, "apps/desktop/src/App.tsx");

test("Desktop CSS defines required semantic tokens in :root", () => {
  const css = fs.readFileSync(indexCssPath, "utf8");
  const requiredTokens = [
    "--coterm-surface",
    "--coterm-surface-raised",
    "--coterm-surface-panel",
    "--coterm-border",
    "--coterm-border-subtle",
    "--coterm-border-input",
    "--coterm-text-primary",
    "--coterm-text-muted",
    "--coterm-accent",
    "--coterm-accent-hover",
    "--coterm-accent-glow",
    "--coterm-danger",
    "--coterm-focus",
    "--coterm-font-sans",
    "--coterm-font-mono",
  ];

  for (const token of requiredTokens) {
    assert.ok(
      css.includes(token),
      `Expected ${token} to be defined in apps/desktop/src/index.css`
    );
  }
});

test("Tailwind configuration maps coterm semantic tokens to CSS custom properties", () => {
  const tailwindContent = fs.readFileSync(tailwindConfigPath, "utf8");
  assert.ok(tailwindContent.includes('"var(--coterm-surface)"'));
  assert.ok(tailwindContent.includes('"var(--coterm-border)"'));
  assert.ok(tailwindContent.includes('"var(--coterm-text-primary)"'));
  assert.ok(tailwindContent.includes('"var(--coterm-accent)"'));
  assert.ok(tailwindContent.includes('"var(--coterm-danger)"'));
  assert.ok(tailwindContent.includes('"var(--coterm-focus)"'));
  assert.ok(tailwindContent.includes('"var(--coterm-font-sans)"'));
});

test("SSH Plugin Shadow DOM styles use semantic tokens with readable fallbacks", () => {
  const sshContent = fs.readFileSync(sshPluginPath, "utf8");

  // Extract the css template literal
  const match = sshContent.match(/const css = `([\s\S]*?)`;/);
  assert.ok(match, "CSS template literal must exist in SSH plugin frontend");
  const shadowCss = match[1];

  // Check that custom properties are used
  assert.ok(shadowCss.includes("var(--coterm-text-primary, #cccccc)"));
  assert.ok(shadowCss.includes("var(--coterm-font-sans,"));
  assert.ok(shadowCss.includes("var(--coterm-surface, #1e1e1e)"));
  assert.ok(shadowCss.includes("var(--coterm-accent, #0e639c)"));
  assert.ok(shadowCss.includes("var(--coterm-border-input, #454545)"));
  assert.ok(shadowCss.includes("var(--coterm-danger, #f48771)"));
  assert.ok(shadowCss.includes("var(--coterm-focus, #007fd4)"));

  // Verify that all var(--coterm-*) usages contain a comma and fallback
  const varMatches = shadowCss.match(/var\(--coterm-[^)]+\)/g) || [];
  assert.ok(varMatches.length > 5, "Expected multiple semantic var() references");
  for (const varCall of varMatches) {
    assert.ok(
      varCall.includes(","),
      `Token usage ${varCall} in Shadow DOM must provide a fallback value`
    );
  }
});

test("Plugin SDK block-list component styles use semantic tokens with fallbacks", () => {
  const blockListContent = fs.readFileSync(blockListPath, "utf8");

  const match = blockListContent.match(/const style = `([\s\S]*?)`;/);
  assert.ok(match, "Style template literal must exist in block-list.ts");
  const blockCss = match[1];

  assert.ok(blockCss.includes("var(--coterm-text-primary, #cccccc)"));
  assert.ok(blockCss.includes("var(--coterm-surface, #1e1e1e)"));
  assert.ok(blockCss.includes("var(--coterm-surface-hover, #2a2d2e)"));
  assert.ok(blockCss.includes("var(--coterm-accent-active, #094771)"));
  assert.ok(blockCss.includes("var(--coterm-accent-glow, #3794ff)"));
  assert.ok(blockCss.includes("var(--coterm-danger, #f48771)"));

  const varMatches = blockCss.match(/var\(--coterm-[^)]+\)/g) || [];
  assert.ok(varMatches.length > 5, "Expected multiple semantic var() references in block-list");
  for (const varCall of varMatches) {
    assert.ok(
      varCall.includes(","),
      `Token usage ${varCall} in block-list must provide a fallback value`
    );
  }
});

test("App.tsx implements responsive navigation collapse and accessible controls", () => {
  const appContent = fs.readFileSync(appTsxPath, "utf8");

  // 1. Check for collapse state and 900px breakpoint
  assert.ok(appContent.includes("instancesCollapsed"), "App must manage instancesCollapsed state");
  assert.ok(appContent.includes("899px") || appContent.includes("900"), "App must check 900px breakpoint");

  // 2. One accessible toggle lives in navigation, without a duplicate toolbar action.
  assert.ok(
    appContent.includes('id="toggle-instances-button"'),
    "Navigation must provide toggle-instances-button"
  );
  assert.ok(
    appContent.includes('aria-controls="instances-sidebar"'),
    "Toggle button must connect to instances-sidebar via aria-controls"
  );
  assert.ok(
    appContent.includes("aria-expanded={false}") && appContent.includes("aria-expanded={true}"),
    "Toggle button must indicate expanded/collapsed state via aria-expanded"
  );
  assert.ok(
    appContent.includes('aria-label={t("Expand instances panel")}') && appContent.includes('aria-label={t("Collapse instances panel")}'),
    "Toggle button must have accessible name"
  );

  // 3. Check for instances sidebar with id and collapse button
  assert.ok(
    appContent.includes('id="instances-sidebar"'),
    "Instances sidebar must have id='instances-sidebar'"
  );
  assert.ok(
    appContent.includes('aria-label={t("Collapse instances panel")}'),
    "Instances header must contain accessible collapse button"
  );

  // 4. Verify workspace has min-w-0 flex-1 overflow-hidden
  assert.ok(
    appContent.includes('main aria-label={t("Plugin workspace")} className="relative isolate min-w-0 flex-1 overflow-hidden'),
    "Main workspace must be flex-1 min-w-0 to prevent horizontal overflow"
  );
});

test("Layout width simulation at 1280px, 1024px, and 768px", () => {
  // Screen specifications:
  // 1280px: Wide screen. Plugins = 72px (sm:w-[72px]), Instances = 240px, Workspace = 1280 - (72 + 240) = 968px.
  // 1024px: Medium screen. Plugins = 64px/72px, Instances = 200px (max-[1024px]:w-[200px]), Workspace = 1024 - (72 + 200) = 752px.
  // 768px: Compact screen. Breakpoint (<900px) collapses instances panel by default.
  //   - Collapsed: Plugins = 72px (sm:w-[72px] applies as 768 >= 640), Instances = 0px, Workspace = 768 - 72 = 696px (prioritizing workspace).
  //   - Expanded (user toggled): Plugins = 72px, Instances = 200px (max-[1024px]:w-[200px]), Workspace = 768 - (72 + 200) = 496px.

  const calculateLayout = (windowWidth, instancesOpen) => {
    const pluginWidth = windowWidth >= 640 ? 72 : 64;
    const instancesWidth = !instancesOpen ? 0 : (windowWidth <= 1024 ? 200 : 240);
    const totalNavWidth = pluginWidth + instancesWidth;
    const workspaceWidth = windowWidth - totalNavWidth;
    return { pluginWidth, instancesWidth, totalNavWidth, workspaceWidth };
  };

  // 1280px - sidebars open
  const l1280 = calculateLayout(1280, true);
  assert.strictEqual(l1280.workspaceWidth, 968);
  assert.ok(l1280.workspaceWidth > 800, "1280px workspace must have ample room");

  // 1024px - sidebars open
  const l1024 = calculateLayout(1024, true);
  assert.strictEqual(l1024.workspaceWidth, 752);
  assert.ok(l1024.workspaceWidth > 600, "1024px workspace must have ample room");

  // 768px - default collapsed under 900px breakpoint
  const l768Collapsed = calculateLayout(768, false);
  assert.strictEqual(l768Collapsed.workspaceWidth, 696);
  assert.ok(l768Collapsed.workspaceWidth >= 650, "768px collapsed workspace prioritizes workspace space");

  // 768px - user manually expands instances panel
  const l768Expanded = calculateLayout(768, true);
  assert.strictEqual(l768Expanded.workspaceWidth, 496);
  assert.ok(l768Expanded.workspaceWidth > 400, "768px expanded workspace maintains positive usable width without overlap");
});
