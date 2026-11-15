import { Button } from "../../../components/ui/button";
import { mountReactContent } from "../react-content";
import type { Format } from "./format-switch";
import type { Translator } from "./i18n";

export interface SendHistory { text: string; format: Format; ending: string; bytes: number }

/** Only this in-memory leaf changes when the bounded send history changes. */
export function mountSendHistory(container: HTMLElement) {
  const view = mountReactContent(container);
  return {
    update(items: SendHistory[], t: Translator, select: (item: SendHistory) => void) {
      view.render(<>{!items.length && <p className="hint">{t("No send history yet.")}</p>}{items.map((item, index) => <Button variant="ghost" size="sm" key={index} onClick={() => select(item)}>
        {`${item.format === "hex" ? "HEX" : "ASCII"} · ${item.text.slice(0, 120).replace(/\s+/g, " ")}`}
      </Button>)}</>);
    }, unmount: view.unmount,
  };
}
