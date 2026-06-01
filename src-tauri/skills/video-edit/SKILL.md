---
name: video-edit
description: Edit, transform, trim, convert, resize, retime, or analyze video and audio files with ffmpeg/ffprobe on the command line. Use when the user wants to cut or trim a clip, change container/format or codec, scale or crop dimensions, change speed or frame rate, extract or replace or mix audio, concatenate clips, burn in or add subtitles/captions/text overlays, extract still frames, make a GIF, adjust volume, or inspect a media file's streams and duration. Operates on existing media files already in the workspace and always produces a NEW output file — it never overwrites the original.
---

# Video & Audio Editing (ffmpeg)

You can edit video and audio in this workspace using `ffmpeg` and `ffprobe`, which
are available on your `PATH`. The workspace folder is your working directory; the
media files the user is pointing at live here (the user's message names the
reference file paths).

This is deterministic editing with a command-line tool — not a generative model.
Read the source with `ffprobe`, run one or more `ffmpeg` commands, and write a new
file. That is the whole job.

## Non-negotiable rules

1. **Never overwrite the original.** Always write a **new** output file. Treat every
   source file as immutable. If `ffmpeg` would write to the input path, choose a
   different output path instead.
2. **Name outputs descriptively** next to the source, e.g. `clip.mp4` →
   `clip-trimmed.mp4`, `clip-720p.mp4`, `clip-square.mp4`, `clip.gif`. Keep the
   output in the workspace (the current directory or a subfolder of it), never in a
   temp dir outside it.
3. **Self-verify with ffprobe.** After producing the output, run `ffprobe` on it and
   confirm the result matches the intent (duration, dimensions, codec, has-audio,
   etc.). If it's wrong, fix it before reporting success.
4. **Report the output path.** End your turn by stating the **absolute path** of the
   new file you created, on its own, so it can be placed on the canvas. If you made
   several outputs, list each absolute path.
5. **Don't touch `.cameo/`.** That dot-folder is Cameo's private Board state — read
   nothing from it and write nothing into it.

## Inspect first

Always probe the source before editing — it tells you dimensions, duration, frame
rate, and whether there's an audio stream, so your command is correct the first time.

```bash
ffprobe -v error -show_format -show_streams -of json input.mp4
# quick duration only:
ffprobe -v error -show_entries format=duration -of csv=p=0 input.mp4
```

## Common operations

Trim (re-encode for frame accuracy; use `-c copy` only when cutting on keyframes):

```bash
ffmpeg -i input.mp4 -ss 00:00:05 -to 00:00:12 -c:v libx264 -c:a aac input-trimmed.mp4
```

Scale to a height, preserving aspect ratio (`-2` keeps width even):

```bash
ffmpeg -i input.mp4 -vf "scale=-2:720" -c:a copy input-720p.mp4
```

Crop to a square centered on the frame:

```bash
ffmpeg -i input.mp4 -vf "crop=min(iw\,ih):min(iw\,ih)" input-square.mp4
```

Change playback speed (0.5 = half-speed video; adjust audio tempo to match):

```bash
ffmpeg -i input.mp4 -filter_complex "[0:v]setpts=2.0*PTS[v];[0:a]atempo=0.5[a]" -map "[v]" -map "[a]" input-slow.mp4
```

Convert / re-encode to a web-friendly MP4:

```bash
ffmpeg -i input.mov -c:v libx264 -pix_fmt yuv420p -movflags +faststart -c:a aac input.mp4
```

Extract / replace / mute audio:

```bash
ffmpeg -i input.mp4 -vn -c:a copy input-audio.m4a            # extract audio
ffmpeg -i input.mp4 -an -c:v copy input-muted.mp4            # drop audio
ffmpeg -i input.mp4 -i music.mp3 -map 0:v -map 1:a -c:v copy -shortest input-scored.mp4  # replace audio
```

Extract a still frame at a timestamp:

```bash
ffmpeg -ss 00:00:03 -i input.mp4 -frames:v 1 -q:v 2 frame-at-3s.jpg
```

Make a GIF (palette pass for quality):

```bash
ffmpeg -i input.mp4 -vf "fps=12,scale=480:-1:flags=lanczos,palettegen" palette.png
ffmpeg -i input.mp4 -i palette.png -filter_complex "fps=12,scale=480:-1:flags=lanczos[x];[x][1:v]paletteuse" input.gif
```

Burn in subtitles from an SRT (the font comes from the OS; this reads system fonts):

```bash
ffmpeg -i input.mp4 -vf "subtitles=captions.srt" -c:a copy input-subbed.mp4
```

Concatenate clips of the same codec/resolution (re-encode if they differ):

```bash
printf "file 'a.mp4'\nfile 'b.mp4'\n" > list.txt
ffmpeg -f concat -safe 0 -i list.txt -c copy joined.mp4
```

## When something fails

- If a `-c copy` cut is imprecise or won't open, re-encode (`-c:v libx264 -c:a aac`).
- If the user's player can't open the output, add `-pix_fmt yuv420p -movflags +faststart`.
- If an operation needs a codec/format ffmpeg here doesn't support, say so plainly
  rather than producing a broken file.

Keep the edit minimal and faithful to what was asked — change only what the user
requested, and leave everything else (resolution, codec, audio) as close to the
source as makes sense.
