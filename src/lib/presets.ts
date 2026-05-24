// One-click instruction templates (PRD §7.6, OQ-5). Per decision D1 these are
// just stored prompts — effect depends on Codex's image model, no extra
// engineering for fidelity. Applied to the current reference selection (batch).

export interface Preset {
  id: string;
  label: string;
  prompt: string;
}

export const PRESETS: Preset[] = [
  {
    id: "remove-bg",
    label: "Remove BG",
    prompt:
      "Remove the background, isolating the main subject on a clean white background. Keep the subject itself unchanged.",
  },
  {
    id: "warmer",
    label: "Warmer",
    prompt:
      "Make the image warmer and more golden, like late-afternoon sunlight, keeping the subject and composition the same.",
  },
  {
    id: "cooler",
    label: "Cooler",
    prompt:
      "Give the image a cooler, bluer tone while keeping the subject and composition the same.",
  },
  {
    id: "film",
    label: "Film look",
    prompt:
      "Apply a cinematic film-camera look — subtle grain, gentle contrast, filmic color — while preserving the subject and composition.",
  },
  {
    id: "bw",
    label: "B&W",
    prompt:
      "Convert to a rich black-and-white photograph with strong contrast, preserving detail and composition.",
  },
  {
    id: "enhance",
    label: "Enhance",
    prompt:
      "Enhance the lighting, sharpness, and color so the image looks polished and professional, without changing the subject or composition.",
  },
];
