import { memo } from "react";
import { Button } from "../components/ui/button";
import { Field, FieldLabel } from "../components/ui/field";
import { Input, Select, Textarea } from "../components/ui/input";
import { ConfigurationFields } from "./ConfigurationFields";
import { workbenchDefinition } from "./definitions";
import { driverI18n } from "../i18n";
import { createTranslator } from "./common/stream-workbench/i18n";
import { TerminalSearchView, FormatSwitchView, JobsPanelView } from "./WorkbenchWidgets";
import { StreamToolbarView } from "./common/stream-workbench/toolbar";

/** React owns the shell; the terminal controller owns xterm and protocol subscriptions. */
export const TerminalView = memo(function TerminalView({ kind }: { kind: "ssh" | "local" }) {
  return <div className="layout" data-ui="layout">
    <div className="bar" data-ui="bar">
      <strong data-ui="label">{kind === "ssh" ? "SSH" : "Local"}</strong>
      <span className="state" data-ui="state" />
      <span className="spacer" />
      <Button variant="ghost" size="sm" data-ui={kind === "ssh" ? "connect" : "restart"} disabled={kind === "ssh"} hidden={kind === "local"} />
    </div>
    <TerminalSearchView />
    {kind === "ssh" ? <div className="ssh-workbench" data-ui="workbench">
      <div className="terminal-pane" data-ui="terminalPane">
        <div className="terminal-shell"><div data-ui="gutter" /><div className="terminal" data-ui="terminalContainer" /></div>
        <div className="jobs" data-ui="jobsBar" hidden><span className="jobs-label" data-ui="jobsLabel" /><div className="jobs-list" data-ui="jobsList" role="list" /><Button variant="ghost" size="sm" className="jobs-toggle h-[26px]" data-ui="jobsExpand" /></div>
        <div data-ui="blocksControls"><div className="ssh-blocks-error" data-ui="blocksErrors" hidden><span className="error" role="alert" data-ui="blocksError" /><Button variant="ghost" size="sm" data-ui="blocksRetry" /></div><div className="ssh-block-history" data-ui="blocksHistory" /></div>
      </div>
      <JobsPanelView />
    </div> : <div className="terminal" data-ui="terminalContainer" />}
    <div className="notice" data-ui="notice" role="status" />
  </div>;
});

const EndingOptions = () => <><option value="none">None</option><option value="cr">CR</option><option value="lf">LF</option><option value="crlf">CRLF</option></>;

/** Shared receive/send workbench. UART controls are present only for byte-capable views. */
export const StreamView = memo(function StreamView({ namespace }: { namespace: "serial" | "tcp" | "telnet" }) {
  const definition = workbenchDefinition(`coterm-driver-${namespace}`)!;
  const t = createTranslator(driverI18n);
  const properties = { ...definition.schema.properties };
  if (namespace === "serial") {
    delete properties.port;
  } else {
    properties.prompt = { type: "string", title: "Shell prompt", maxLength: 256 };
  }
  return <div className="layout" data-ui="layout">
    <div className="bar" data-ui="bar">
      <div className="modes" role="group" data-ui="modes"><Button variant="ghost" size="sm" data-ui="terminalMode" /><Button variant="ghost" size="sm" data-ui="uartMode" /></div>
      <span className="spacer" /><Button variant="ghost" size="sm" className="connect-button" data-ui="connect" disabled />
      <Button variant="ghost" size="sm" className="settings-toggle" data-ui="settingsButton" aria-expanded="false" aria-controls={`${namespace}-settings`} />
      <Button variant="ghost" size="icon" className="toolbar-toggle" data-ui="toolbarButton" aria-expanded="false" aria-controls={`${namespace}-toolbar-settings`} title={t("Customize toolbar")}>
        <svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor"><path d="M4 7h7m4 0h5M4 17h2m4 0h10M11 4v6M6 14v6" /></svg>
      </Button>
    </div>
    <TerminalSearchView />
    <div className="receive-area" data-ui="receiveArea">
      <div className="panel" data-ui="terminalPanel"><div className="terminal" data-ui="terminalContainer" /></div>
      <div className="panel uart" data-ui="uartPanel" hidden><StreamToolbarView namespace={namespace} /><pre className="uart-output" data-ui="uartOutput" tabIndex={0} /></div>
      <section id={`${namespace}-settings`} className="settings-panel" data-ui="settingsPanel" tabIndex={-1} aria-hidden="true">
        <form className="communication-settings settings-grid" data-ui="communication">
          <ConfigurationFields schema={{ ...definition.schema, properties }} defaults={definition.defaults} translate={t} manageConditions={false} />
          {namespace === "tcp" && <div hidden><Input name="username" /><Select name="terminalType" defaultValue="xterm"><option value="xterm">xterm</option></Select></div>}
          <div
            hidden={namespace !== "serial"}
            className="receive-newlines col-span-2 grid grid-cols-2 gap-3"
            data-ui="receiveNewlines"
            role="group"
          >
            <label className="signal-check flex h-9 items-center gap-2.5 cursor-pointer select-none rounded-sm px-2.5 border border-input bg-background/80 hover:bg-muted/60 hover:border-border transition-colors">
              <Input type="checkbox" name="receiveCompleteLF" data-ui="completeLF" className="size-4 rounded-sm accent-primary cursor-pointer shrink-0" />
              <span data-ui="completeLFLabel" className="text-xs font-medium text-foreground leading-none truncate" />
            </label>
            <label className="signal-check flex h-9 items-center gap-2.5 cursor-pointer select-none rounded-sm px-2.5 border border-input bg-background/80 hover:bg-muted/60 hover:border-border transition-colors">
              <Input type="checkbox" name="receiveCompleteCR" data-ui="completeCR" className="size-4 rounded-sm accent-primary cursor-pointer shrink-0" />
              <span data-ui="completeCRLabel" className="text-xs font-medium text-foreground leading-none truncate" />
            </label>
          </div>
          {namespace === "serial" && (
            <Field data-field="prompt">
              <FieldLabel htmlFor={`${namespace}-prompt`} className="text-xs font-medium text-muted-foreground truncate">
                {t("Shell prompt")}
              </FieldLabel>
              <Input
                id={`${namespace}-prompt`}
                name="prompt"
                type="text"
                defaultValue="# "
                maxLength={256}
                autoComplete="off"
                spellCheck={false}
                className="h-9 text-xs"
              />
            </Field>
          )}
          <Field data-field="backspace" hidden={namespace !== "serial"}>
            <FieldLabel htmlFor={`${namespace}-backspace-key`} data-ui="backspaceLabel" className="text-xs font-medium text-muted-foreground truncate">
              <span data-ui="backspaceLabelText" />
            </FieldLabel>
            <Select id={`${namespace}-backspace-key`} name="backspaceKey" data-ui="backspaceKey" className="h-9 text-xs"><option value="bs">BS (0x08)</option><option value="del">DEL (0x7F)</option></Select>
          </Field>
          <Field data-field="uartEnding" hidden={namespace === "telnet"}>
            <FieldLabel htmlFor={`${namespace}-uart-ending`} data-ui="uartEndingLabel" className="text-xs font-medium text-muted-foreground truncate">
              <span data-ui="uartEndingLabelText" />
            </FieldLabel>
            <Select id={`${namespace}-uart-ending`} className="line-ending h-9 text-xs" data-ui="uartEnding"><EndingOptions /></Select>
          </Field>
          <Field data-field="receiveFilename" hidden={namespace === "telnet"}>
            <FieldLabel htmlFor={`${namespace}-receive-filename`} data-ui="receiveFilenameLabel" className="text-xs font-medium text-muted-foreground truncate">
              <span data-ui="receiveFilenameLabelText">{t("Receive buffer filename")}</span>
            </FieldLabel>
            <Input id={`${namespace}-receive-filename`} className="receive-filename h-9 text-xs" data-ui="receiveFilename" defaultValue={`${namespace}-receive.txt`} maxLength={128} />
          </Field>
        </form>
      </section>
    </div>
    <div className="uart-send" data-ui="uartSend" hidden={namespace === "telnet"}>
      <div className="entry"><label className="entry"><span data-ui="uartEntryText" hidden /><Textarea data-ui="uartText" rows={3} spellCheck={false} maxLength={49154} /></label>
        <div id={`${namespace}-send-history`} className="send-history" data-ui="historyPanel" hidden role="region" />
      </div>
      <div className="file-actions">
        <Button variant="ghost" size="icon" className="import-send" data-ui="importButton"><svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor"><path d="M3 7V4h6l3 3h9v13H3V7zm0 3h18" /></svg></Button>
        <Button variant="ghost" size="icon" className="history-toggle" data-ui="historyButton" aria-expanded="false" aria-controls={`${namespace}-send-history`}><svg viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor"><path d="M3 12a9 9 0 1 0 2.6-6.4L3 8M3 3v5h5M12 7v5l4 2" /></svg></Button>
        <input type="file" data-ui="fileInput" hidden />
      </div>
      <div className="send-controls" data-ui="sendControls">
        <FormatSwitchView name="sendFormat" send />
        <div className="inline" data-ui="repeatRow"><Input type="number" data-ui="repeatMs" min={20} max={60000} step={10} defaultValue={100} /><span>ms</span><Button variant="ghost" size="sm" className="repeat-button" data-ui="repeatButton" disabled /></div>
      </div>
    </div>
  </div>;
});
