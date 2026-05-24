import { useState } from "react";
import { createPortal } from "react-dom";
import { Images } from "lucide-react";
import { useT } from "../../i18n/locale";
import { CLOUD_ENABLED } from "../../services/cloud";
import { GalleryOverlay } from "./GalleryOverlay";

/**
 * The "Gallery" entry mounted directly above the composer. Renders only in
 * official builds (CLOUD_ENABLED). Open-source self-builds have no button —
 * the whole feature is invisible.
 *
 * The overlay itself is portaled to `document.body` so its `position: fixed`
 * backdrop is relative to the viewport rather than whatever transformed /
 * filtered ancestor the composer happens to live inside (chat panel applies
 * `backdrop-filter`, which silently re-roots the fixed positioning context).
 */
export function GalleryButton() {
  const t = useT();
  const [open, setOpen] = useState(false);

  if (!CLOUD_ENABLED) return null;

  return (
    <>
      <div className="cm-composer__topbar">
        <button
          type="button"
          className="cm-gallery-trigger"
          onClick={() => setOpen(true)}
          aria-label={t("gallery.button")}
        >
          <Images size={14} />
          <span>{t("gallery.button")}</span>
        </button>
      </div>
      {open && createPortal(
        <GalleryOverlay onClose={() => setOpen(false)} />,
        document.body,
      )}
    </>
  );
}
