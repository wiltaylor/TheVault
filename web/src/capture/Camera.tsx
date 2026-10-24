// The capture dialog's camera (capture K1). On a phone, or in the Android app, the Camera button
// opens the phone's camera through a file input with `capture`. On a desktop browser with a
// camera it opens a small preview here, with Take photo, Retake and Use photo. With no camera the
// button does not show.

import { Show, createSignal, onCleanup, onMount } from "solid-js";
import { appInfo } from "../inApp";

/**
 * How this device takes a photo: `capture` through a file input that asks for the camera,
 * `preview` through the camera's stream shown in the dialog, or null with no camera.
 */
export type CameraMode = "capture" | "preview" | null;

/** The longest edge of a photo taken in the preview, in pixels. */
export const PHOTO_MAX_EDGE = 2560;

/** The JPEG quality of a photo taken in the preview. */
const PHOTO_QUALITY = 0.9;

/** Whether the page runs on a phone or tablet: a touch screen as its main pointer. */
function isPhone(): boolean {
  return typeof window !== "undefined" && !!window.matchMedia?.("(pointer: coarse)").matches;
}

/** How this device takes a photo. */
export async function cameraMode(): Promise<CameraMode> {
  if ((await appInfo())?.platform === "android" || isPhone()) return "capture";
  const media = typeof navigator === "undefined" ? undefined : navigator.mediaDevices;
  if (!media?.getUserMedia || !media.enumerateDevices) return null;
  try {
    return (await media.enumerateDevices()).some((d) => d.kind === "videoinput") ? "preview" : null;
  } catch {
    return null;
  }
}

/** The size of a photo of a `width` by `height` frame: the frame's, with its long edge capped at `max`. */
export function photoSize(width: number, height: number, max = PHOTO_MAX_EDGE): { width: number; height: number } {
  const scale = Math.min(1, max / Math.max(width, height, 1));
  return { width: Math.max(1, Math.round(width * scale)), height: Math.max(1, Math.round(height * scale)) };
}

const pad = (n: number) => String(n).padStart(2, "0");

/** A photo's file name, from when it was taken. */
function photoName(d: Date): string {
  return `photo-${d.getFullYear()}${pad(d.getMonth() + 1)}${pad(d.getDate())}-${pad(d.getHours())}${pad(d.getMinutes())}${pad(d.getSeconds())}.jpg`;
}

/**
 * The desktop camera preview: the camera's live picture with Take photo, then the still with
 * Retake and Use photo. Use photo hands over the photo as a JPEG; Cancel, or Escape in the
 * dialog, closes the preview. The camera is let go when the preview closes.
 */
export function CameraPreview(props: { onUse: (photo: File) => void; onClose: () => void }) {
  let video!: HTMLVideoElement;
  let stream: MediaStream | null = null;
  const [ready, setReady] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [shot, setShot] = createSignal<{ blob: Blob; url: string } | null>(null);
  let closed = false;

  function release() {
    for (const t of stream?.getTracks() ?? []) t.stop();
    stream = null;
  }

  onMount(async () => {
    try {
      const s = await navigator.mediaDevices.getUserMedia({
        video: { facingMode: { ideal: "environment" }, width: { ideal: 4096 }, height: { ideal: 4096 } },
        audio: false,
      });
      if (closed) {
        for (const t of s.getTracks()) t.stop();
        return;
      }
      stream = s;
      video.srcObject = s;
      await video.play?.()?.catch(() => undefined);
      setReady(true);
    } catch (e) {
      setError(e instanceof Error && e.name === "NotAllowedError" ? "The browser was not allowed to use the camera." : "The camera could not be opened.");
    }
  });

  onCleanup(() => {
    closed = true;
    release();
    const s = shot();
    if (s) URL.revokeObjectURL(s.url);
  });

  function take() {
    const settings = stream?.getVideoTracks()[0]?.getSettings?.() ?? {};
    const size = photoSize(video.videoWidth || settings.width || 1280, video.videoHeight || settings.height || 720);
    const canvas = document.createElement("canvas");
    canvas.width = size.width;
    canvas.height = size.height;
    const ctx = canvas.getContext("2d");
    if (!ctx) {
      setError("The photo could not be taken.");
      return;
    }
    ctx.drawImage(video, 0, 0, size.width, size.height);
    canvas.toBlob(
      (blob) => {
        if (!blob) return setError("The photo could not be taken.");
        setShot({ blob, url: URL.createObjectURL(blob) });
      },
      "image/jpeg",
      PHOTO_QUALITY,
    );
  }

  function retake() {
    const s = shot();
    if (s) URL.revokeObjectURL(s.url);
    setShot(null);
  }

  function use() {
    const s = shot();
    if (!s) return;
    props.onUse(new File([s.blob], photoName(new Date()), { type: "image/jpeg" }));
  }

  return (
    <div class="capture-camera" role="group" aria-label="Camera">
      <div class="capture-camera-frame">
        <video ref={video} class="capture-camera-video" aria-label="Camera preview" autoplay muted playsinline hidden={!!shot()} />
        <Show when={shot()}>{(s) => <img class="capture-camera-video" src={s().url} alt="Photo taken" />}</Show>
      </div>
      <Show when={error()}>
        <p class="error" role="alert">
          {error()}
        </p>
      </Show>
      <div class="capture-actions">
        <button type="button" onClick={props.onClose}>
          Cancel
        </button>
        <span class="capture-spacer" />
        <Show
          when={shot()}
          fallback={
            <button type="button" class="primary" disabled={!ready()} onClick={take}>
              Take photo
            </button>
          }
        >
          <button type="button" onClick={retake}>
            Retake
          </button>
          <button type="button" class="primary" onClick={use}>
            Use photo
          </button>
        </Show>
      </div>
    </div>
  );
}
