import { Button } from "../components/ui/button";
import { Input } from "../components/ui/input";

/** Search markup is shared; xterm owns matching and selection through its adapter. */
export function TerminalSearchView() {
  return <div className="terminal-find" role="search" hidden>
    <Input type="search" maxLength={512} spellCheck={false} autoComplete="off" />
    <span className="terminal-find-result" role="status" aria-live="polite" />
    <Button variant="ghost" size="sm" data-search="previous">↑</Button>
    <Button variant="ghost" size="sm" data-search="next">↓</Button>
    <Button variant="ghost" size="sm" data-search="close">×</Button>
  </div>;
}

/** Stable nodes keep focus when the controller changes receive or send format. */
export function FormatSwitchView({ name, send = false }: { name: string; send?: boolean }) {
  return <div className={`format-switch${send ? " send-mode" : ""}`} role="group" data-ui={name}>
    <span className="format-thumb" aria-hidden="true" />
    <Button variant="ghost" size="sm" data-value="text">ASCII</Button><Button variant="ghost" size="sm" data-value="hex">HEX</Button>
  </div>;
}

/** SSH keeps its own background-job model while sharing standard host controls. */
export function JobsPanelView() {
  return <>
    <div className="exec-splitter" data-ui="jobSplitter" role="separator" tabIndex={0} hidden />
    <section className="job-view" data-ui="jobPanel" tabIndex={-1} hidden>
      <div className="job-head"><strong>Exec</strong><span className="job-state" data-ui="jobState" role="status" />
        <Button variant="ghost" size="sm" className="job-dock" data-ui="jobDock" />
        <Button variant="ghost" size="icon" className="job-close" data-ui="jobClose"><svg viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true"><path d="m3 3 6 6M9 3 3 9" /></svg></Button>
      </div>
      <div className="job-follow" data-ui="jobFollow" hidden><span data-ui="jobFollowLabel" /><Button variant="ghost" size="sm" data-ui="jobResume" /></div>
      <div className="job-body" data-ui="jobBody" tabIndex={0}>
        <section className="job-command-section"><h4 data-ui="jobCommandLabel" /><pre className="job-command" data-ui="jobCommand" /></section>
        {(["stdout", "stderr"] as const).map(name => <section key={name} className="job-stream" data-stream={name} aria-label={name.toUpperCase()}>
          <h4>{name.toUpperCase()}</h4><pre data-ui={`job-${name}`} />
        </section>)}
      </div>
    </section>
  </>;
}
