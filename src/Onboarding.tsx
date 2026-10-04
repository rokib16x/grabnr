import { useEffect, useState } from "react";
import { api } from "./api";
import { linkColor } from "./format";
import { Icon, linkIcon } from "./icons";
import type { AppState, LinksResponse } from "./types";
import { MOD, TRAY } from "./platform";
import { t } from "./i18n";

/** First-run setup: what grabnr does, whether your connections are ready, and how to connect the browser. */
export function Onboarding({ app, onDone }: { app: AppState; onDone: () => void }) {
  const [step, setStep] = useState(0);
  const [links, setLinks] = useState<LinksResponse | null>(null);

  useEffect(() => void api.links().then(setLinks), []);

  const finish = async () => {
    await api.setSettings({ onboarded: true });
    onDone();
  };
  const usable = links?.links.filter((l) => l.kind !== "tunnel") ?? [];
  const shared = links?.shared_gateways ?? [];

  const steps = [
    <>
      <img src="/grabnr.svg" alt={t("")} width="72" height="72" />
      <h2>{t("Welcome to grabnr")}</h2>
      <p>{t("grabnr downloads a file over every connection you have at once: Wi-Fi, Ethernet, USB or phone tethering. The file is split into pieces and each connection pulls pieces as fast as it can, so faster links carry more.")}</p>
    </>,
    <>
      <h2>{t("Your connections")}</h2>
      {!links && <p className="muted">{t("Looking for connections…")}</p>}
      {links && usable.length === 0 && <p className="check bad"><Icon name="alert" />{t("No active connection found. Downloads will use the system default route.")}</p>}
      {links && usable.length === 1 && <p className="check warn"><Icon name="alert" />{t("Only one connection found. grabnr works, but a second one (for example a phone hotspot or Ethernet) is what makes downloads faster.")}</p>}
      {usable.length > 1 && <p className="check ok"><Icon name="check" />{t("{n} connections ready.", { n: usable.length })}</p>}
      <ul className="conns big">
        {usable.map((l) => (
          <li key={l.name}><i className="tile" style={{ background: linkColor(l.name) }}><Icon name={linkIcon(l.kind)} size={14} /></i>{l.label}<small>{l.ipv4}</small></li>
        ))}
      </ul>
      {links?.binding === "source_address" && <p className="check warn"><Icon name="alert" />{t("This system steers each download to a connection by its address, which works when every connection has its own router. Use Test links in Manage Connections to confirm.")}</p>}
      {shared.map(([a, b]) => <p key={a + b} className="check warn"><Icon name="alert" />{t("{a} and {b} go through the same router, so using both will not add speed.", { a, b })}</p>)}
      <p className="muted small">{t("You can turn connections on or off, or limit them, in Manage Connections.")}</p>
    </>,
    <>
      <h2>{t("Browser downloads")}</h2>
      <p>{t("Install the grabnr extension in Chrome, Brave or Edge so downloads start here instead of in the browser. Open the browser's extensions page, turn on developer mode, and load the extension folder from the grabnr project. There is nothing to pair: grabnr recognises its own extension.")}</p>
      <p className={`check ${app.api_ok ? "ok" : "bad"}`}><Icon name={app.api_ok ? "check" : "alert"} />{app.api_ok ? t("Ready for the extension on port {port}.", { port: app.api_port }) : t("Port {port} is used by another program, so the extension cannot connect.", { port: app.api_port })}</p>
    </>,
    <>
      <h2>{t("Lives in the {tray}", { tray: t(TRAY) })}</h2>
      <p>{t("Closing the window keeps grabnr running so downloads continue. Click the {tray} icon to see progress, pause everything, or open the window. You can also paste a link anywhere in the window to start a download, or press {key}N.", { tray: t(TRAY), key: MOD })}</p>
    </>,
  ];

  return (
    <div className="modal" role="dialog" aria-label={t("Welcome to grabnr")}>
      <div className="sheet onboard">
        <div className="onboard-body">{steps[step]}</div>
        <div className="dots" aria-hidden="true">{steps.map((_, i) => <span key={i} className={i === step ? "on" : ""} />)}</div>
        <div className="sheet-actions">
          {step === 0 ? <button className="ghost" onClick={finish}>{t("Skip")}</button> : <button onClick={() => setStep(step - 1)}>{t("Back")}</button>}
          {step < steps.length - 1 ? <button className="primary" onClick={() => setStep(step + 1)}>{t("Continue")}</button> : <button className="primary" onClick={finish}>{t("Start using grabnr")}</button>}
        </div>
      </div>
    </div>
  );
}
