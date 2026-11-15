/** Resolve one React-owned view node. Missing markup fails before installing subscriptions. */
export function viewNodes(container: HTMLElement) {
  return <T extends HTMLElement>(name: string): T => {
    const node = container.querySelector<T>(`[data-ui="${name}"]`);
    if (!node) throw new Error(`Missing workbench view: ${name}`);
    return node;
  };
}
