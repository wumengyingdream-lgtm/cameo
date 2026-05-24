import { isPermissionGranted, requestPermission, sendNotification } from "@tauri-apps/plugin-notification";

let permission: boolean | null = null;

/** Notify the OS that a turn finished — only when the window is unfocused, so a
 *  user actively watching the canvas is never interrupted. Permission is asked
 *  lazily on the first unfocused completion. */
export async function notifyTurnDone(body: string): Promise<void> {
  if (typeof document !== "undefined" && document.hasFocus()) return;
  try {
    if (permission === null) {
      permission = await isPermissionGranted();
      if (!permission) permission = (await requestPermission()) === "granted";
    }
    if (permission) sendNotification({ title: "Cameo", body });
  } catch {
    /* notifications are best-effort */
  }
}
