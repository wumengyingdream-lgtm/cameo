import { RotateCw, Loader2, TriangleAlert } from "lucide-react";
import { useT } from "../i18n/locale";
import { useUpdater } from "../hooks/useUpdater";

/**
 * Topbar "重启更新" button. Renders only when there's a pending update on
 * disk (silent download has completed). One click → graceful Codex sidecar
 * shutdown → install + relaunch.
 *
 * Mounted in the topbar to the LEFT of the Settings gear so users see it
 * exactly where they're already looking when they click anything in the
 * top-right cluster.
 */
export function UpdateIndicator() {
  const t = useT();
  const { pendingVersion, installing, error, restart } = useUpdater();

  if (!pendingVersion) return null;

  return (
    <button
      type="button"
      className={`cm-update-btn${error ? " is-error" : ""}`}
      title={error ? t("update.errorTooltip", { error }) : t("update.tooltip", { version: pendingVersion })}
      onClick={() => void restart()}
      disabled={installing}
    >
      {installing ? (
        <Loader2 size={13} className="cm-update-btn__spin" />
      ) : error ? (
        <TriangleAlert size={13} />
      ) : (
        <RotateCw size={13} />
      )}
      <span>{error ? t("update.retryButton") : t("update.button")}</span>
    </button>
  );
}
