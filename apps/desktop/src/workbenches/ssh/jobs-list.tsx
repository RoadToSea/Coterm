import { useLayoutEffect } from "react";
import { Button } from "../../components/ui/button";
import { mountReactContent } from "../common/react-content";
import type { Translator } from "../common/stream-workbench/i18n";

interface Item { id: string; text: string; name: string; state: string; index?: number }
interface Props {
  items: Item[]; selected: string | null; expanded: boolean; contextIds: Set<string>; t: Translator;
  toggle(id: string): void; select(id: string): void; dismiss(id: string): void; layout(): void;
}
function JobsList({ items, selected, expanded, contextIds, t, toggle, select, dismiss, layout }: Props) {
  useLayoutEffect(layout);
  let after = false;
  return <>{items.map(job => {
    const hidden = !expanded && after;
    if (selected === job.id) after = true;
    return <div key={job.id} className="job" role="listitem" data-operation-id={job.id} hidden={hidden} data-selected={String(selected === job.id)}>
      <Button variant="ghost" size="sm" className="job-block-number h-6" hidden={job.index === undefined} aria-pressed={contextIds.has(job.id)}
        aria-label={t("Select block #{index} for Agent", { index: job.index ?? "?" })} onClick={() => toggle(job.id)}>{job.index}</Button>
      <Button variant="ghost" size="sm" className="job-select h-6" title={job.text} aria-current={selected === job.id} aria-label={`${job.text} · ${t(job.state)}`} onClick={() => select(job.id)}>
        <span className="job-led" aria-hidden="true" data-state={job.state} /><span className="name">{job.name}</span>
      </Button>
      <Button variant="ghost" size="icon" className="dismiss" aria-label={`${t("Remove job from list")}: ${job.name}`} onClick={() => dismiss(job.id)}>
        <svg viewBox="0 0 12 12" fill="none" stroke="currentColor" aria-hidden="true"><path d="m3 3 6 6M9 3 3 9" /></svg>
      </Button>
    </div>;
  })}</>;
}

/** Job updates reconcile keyed buttons so polling cannot steal keyboard focus. */
export function mountJobsList(container: HTMLElement) {
  const view = mountReactContent(container);
  return { update(props: Props) { view.render(<JobsList {...props} />); }, unmount: view.unmount };
}
