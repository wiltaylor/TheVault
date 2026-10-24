// The app builds this server hands out: on Settings > Devices, and on the public /apps page, which
// works without signing in so a phone can fetch the Android app before it is installed.

import { A } from "@solidjs/router";
import { For, Show, createResource, createSignal } from "solid-js";
import { type ClientBuild, api } from "../api";
import { when } from "../admin/format";
import { copyText } from "../clipboard";
import { formatSize } from "../blocks/MediaBlocks";
import { sessionState } from "../session";
import { errorText } from "./TypeEditor";
import { needsServer } from "../inApp";

const KIND_LABELS: Record<ClientBuild["kind"], string> = {
  apk: "APK",
  appimage: "AppImage",
  deb: ".deb",
  rpm: ".rpm",
};

const PLATFORMS: { platform: ClientBuild["platform"]; title: string; blurb: string }[] = [
  {
    platform: "android",
    title: "Android",
    blurb: "Scan the code with the phone's camera, or open this page on the phone, then install the APK.",
  },
  {
    platform: "linux",
    title: "Linux desktop",
    blurb: "The AppImage runs on any distribution: make it executable and start it. A .deb or .rpm installs the app system-wide.",
  },
];

/** The builds of one platform, Android's APK or the Linux AppImage first. */
export function buildsFor(builds: readonly ClientBuild[], platform: ClientBuild["platform"]): ClientBuild[] {
  const order: ClientBuild["kind"][] = ["apk", "appimage", "deb", "rpm"];
  return builds.filter((b) => b.platform === platform).sort((a, b) => order.indexOf(a.kind) - order.indexOf(b.kind));
}

/** The absolute address of a build, for its QR code and for copying. */
function absolute(b: ClientBuild): string {
  return b.url ?? new URL(b.path, window.location.origin).toString();
}

/** The SHA-256 of a build, with a button that copies it. */
function Checksum(props: { build: ClientBuild }) {
  const [copied, setCopied] = createSignal<"idle" | "copied" | "failed">("idle");
  async function copy() {
    setCopied((await copyText(props.build.sha256)) ? "copied" : "failed");
    setTimeout(() => setCopied("idle"), 1500);
  }
  return (
    <div class="client-sha">
      <span class="muted">SHA-256</span>
      <code aria-label={`SHA-256 of ${props.build.filename}`}>{props.build.sha256}</code>
      <button type="button" class="client-copy" onClick={() => void copy()}>
        {copied() === "copied" ? "Copied" : copied() === "failed" ? "Copy failed" : "Copy"}
      </button>
    </div>
  );
}

function BuildRow(props: { build: ClientBuild }) {
  const b = props.build;
  return (
    <div class="client-build" data-kind={b.kind} aria-label={`${KIND_LABELS[b.kind]} ${b.version}`}>
      <div class="client-build-head">
        <strong>{KIND_LABELS[b.kind]}</strong>
        <span>{b.version}</span>
        <span class={`client-badge ${b.build}`} title={b.build === "debug" ? "A debug build, signed with a debug key" : "A signed release build"}>
          {b.build}
        </span>
        <span class="muted">
          {b.arch} · {formatSize(b.size)} · published {when(b.published)}
        </span>
      </div>
      <div class="client-build-actions">
        <a class={`button client-download${b.kind === "apk" || b.kind === "appimage" ? " primary" : ""}`} href={b.path} download={b.filename}>
          Download {KIND_LABELS[b.kind]}
        </a>
        <span class="muted client-filename">{b.filename}</span>
      </div>
      <Checksum build={b} />
    </div>
  );
}

function PlatformCard(props: { title: string; blurb: string; builds: ClientBuild[] }) {
  const apk = () => props.builds.find((b) => b.kind === "apk");
  return (
    <article class="field-card client-card" aria-label={props.title}>
      <h3>{props.title}</h3>
      <div class="client-card-body">
        <Show when={apk()}>
          {(a) => (
            <figure class="client-qr">
              <Show
                when={a().qr_svg}
                fallback={<div class="client-qr-missing muted">No QR code: the server could not tell its own address.</div>}
              >
                {(svg) => (
                  <img
                    alt={`QR code of the ${a().version} APK download`}
                    src={`data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg())}`}
                  />
                )}
              </Show>
              <figcaption class="muted">
                <code>{absolute(a())}</code>
              </figcaption>
            </figure>
          )}
        </Show>
        <div class="client-builds">
          <p class="muted">{props.blurb}</p>
          <For each={props.builds}>{(b) => <BuildRow build={b} />}</For>
        </div>
      </div>
    </article>
  );
}

/**
 * Every build the server hands out, by platform. With nothing published it says how to publish:
 * `just publish-clients <dir>` into the server's clients folder.
 */
export function ClientDownloads(props: { heading?: "h1" | "h2" }) {
  const [answer] = createResource(api.clients);
  // A resource that failed throws when read, so the answer is read only once it is ready.
  const ready = () => (answer.state === "ready" ? answer() : undefined);
  const builds = () => ready()?.clients;
  const heading = () => props.heading ?? "h2";
  // A server with no clients folder hands out no builds, and an app that cannot reach its home
  // server has none to show: there is nothing to offer or explain.
  return (
    <Show when={ready()?.enabled !== false && !needsServer(answer.error)}>
      <section class="client-downloads" aria-label="Get the apps">
        <Show when={heading() === "h1"} fallback={<h2>Get the apps</h2>}>
          <h1>Get the apps</h1>
        </Show>
        <Show when={answer.error}>
          <p class="error" role="alert">
            Could not list the apps: {errorText(answer.error)}
          </p>
        </Show>
        <Show when={builds()}>
          {(list) => (
            <Show
              when={list().length > 0}
              fallback={
                <div class="client-empty">
                  <p>This server has no app builds to hand out yet.</p>
                  <p class="muted">
                    Build the apps, then copy them into the server's clients folder with{" "}
                    <code>just publish-clients &lt;dir&gt;</code>, where <code>&lt;dir&gt;</code> is the server's{" "}
                    <code>--clients</code> folder (by default <code>&lt;vault&gt;/.vault/clients</code>). They show here
                    at once.
                  </p>
                </div>
              }
            >
              <div class="client-cards">
                <For each={PLATFORMS.filter((p) => buildsFor(list(), p.platform).length > 0)}>
                  {(p) => <PlatformCard title={p.title} blurb={p.blurb} builds={buildsFor(list(), p.platform)} />}
                </For>
              </div>
            </Show>
          )}
        </Show>
      </section>
    </Show>
  );
}

/** `/apps`: the downloads on a page of their own, which needs no session. */
export function AppsPage() {
  return (
    <main class="apps-page">
      <header class="apps-page-head">
        <span class="apps-page-brand">TheVault</span>
        <Show when={sessionState() === "in"} fallback={<A href="/login">Log in</A>}>
          <A href="/">Open TheVault</A>
        </Show>
      </header>
      <ClientDownloads heading="h1" />
    </main>
  );
}
