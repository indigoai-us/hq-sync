#!/usr/bin/env node
/**
 * recall-sdk-bridge — adapter from @recallai/desktop-sdk (Node.js callbacks)
 * to the ndjson-over-stdout protocol that hq-sync's recall_sdk.rs expects.
 *
 * The real Recall Desktop SDK is a Node.js library, not a CLI. This bridge
 * runs as a Tauri sidecar child process under hq-sync, requires the SDK,
 * subscribes to its `meeting-detected` events, and translates each event
 * into the schema documented in repos/private/hq-sync/src-tauri/src/events.rs.
 *
 * --- Wire format expected by recall_sdk.rs ---
 *
 *   {"type":"meeting:detected","detectionId":"<uuid>","meetingUrl":"<url>",
 *    "platform":"zoom|meet|teams|slack|webex|other",
 *    "detectedAt":"<ISO 8601>","source":"sdk-active-app"}
 *
 * --- What the real SDK provides ---
 *
 *   addEventListener('meeting-detected', (e) => {
 *     // e.window = { id, title?, url?, platform? }
 *   })
 *
 * --- Synthesized fields ---
 *
 *   detectionId  — UUID v4 per event (real SDK doesn't surface a stable id)
 *   detectedAt   — current ISO 8601 timestamp at event receive time
 *   source       — hardcoded "sdk-active-app"; correlating with the local
 *                  calendar to mark "sdk-calendar" is a future enhancement
 *   sourceEventId — omitted; same future-enhancement note
 *
 * --- API key ---
 *
 *   Read from env RECALL_API_KEY. The launching Rust sidecar (recall_sdk.rs)
 *   fetches the key from hq-pro GET /v1/recall/credentials and passes it
 *   via env on spawn.
 *
 * --- Lifecycle ---
 *
 *   On SIGTERM (sent by recall_sdk.rs on app shutdown via cancel_process_impl),
 *   call RecallAiSdk.shutdown() then exit 0 within 5s.
 */

import { randomUUID } from "node:crypto";
import { createRequire } from "node:module";
import { createInterface } from "node:readline";

const require = createRequire(import.meta.url);

// Load the SDK lazily so a missing/broken install surfaces a clean error to
// stderr (captured by recall_sdk.rs as RECALL_SDK_UNAVAILABLE) rather than
// crashing import.
let RecallAiSdk;
try {
  RecallAiSdk = require("@recallai/desktop-sdk").default;
} catch (err) {
  console.error("[recall-sdk-bridge] failed to load @recallai/desktop-sdk:", err.message);
  process.exit(2);
}

// --- Helpers ---

/**
 * Map the SDK's free-form `platform` string to our enum.
 * The SDK uses lowercase identifiers; unknown values fall through to "other".
 */
function normalisePlatform(p) {
  if (typeof p !== "string") return "other";
  const lc = p.trim().toLowerCase();
  if (["zoom", "meet", "teams", "slack", "webex"].includes(lc)) return lc;
  if (lc === "googlemeet" || lc === "google-meet") return "meet";
  if (lc === "msteams" || lc === "microsoft-teams") return "teams";
  return "other";
}

function emitNdjson(obj) {
  // One JSON object per line, flushed immediately. process.stdout is a TTY
  // when run interactively but a pipe when spawned by recall_sdk.rs — both
  // honour synchronous writes via write+drain semantics. Use `\n` not
  // `os.EOL` because the Rust side reads byte-stream lines.
  process.stdout.write(JSON.stringify(obj) + "\n");
}

function emitLog(level, message) {
  // Bridge-side diagnostics go to stderr; recall_sdk.rs tags them as
  // "recall-sdk.stderr" log entries in ~/.hq/sync-debug.log.
  console.error(`[recall-sdk-bridge] ${level}: ${message}`);
}

// --- Boot ---

const apiKey = process.env.RECALL_API_KEY;
if (!apiKey) {
  emitLog("error", "RECALL_API_KEY env var is required");
  process.exit(3);
}

// Region must match the API key's region. The Indigo Recall account is
// in us-west-2 (the bot path under hq-pro hits us-west-2 too); a deploy
// briefly targeted us-east-1 on 2026-05-26 and Recall rejected with
// HTTP 401 + "Invalid API token … might be for another Recall region".
// Override via RECALL_API_URL env if the account is ever migrated.
const apiUrl = process.env.RECALL_API_URL || "https://us-west-2.recall.ai";
const dev = process.env.RECALL_SDK_DEV === "1";

emitLog("info", `starting SDK init (apiUrl=${apiUrl} dev=${dev})`);

// The five permissions the SDK can request on macOS. We track all of them so
// the Svelte UI can render an exact "missing permissions" list and deep-link
// the user into System Settings for each.
const REQUIRED_PERMISSIONS = [
  "accessibility",
  "screen-capture",
  "microphone",
  "system-audio",
  "full-disk-access",
];

try {
  await RecallAiSdk.init({
    apiUrl,
    dev,
    // Acquire on startup so the user gets the standard macOS prompts on
    // first run. After first run macOS won't re-prompt; the Svelte UI surfaces
    // missing permissions and deep-links into System Settings.
    acquirePermissionsOnStartup: REQUIRED_PERMISSIONS,
    restartOnError: true,
  });
} catch (err) {
  emitLog("error", `SDK init failed: ${err?.message ?? err}`);
  process.exit(4);
}

emitLog("info", "SDK init complete; listening for meeting-detected events");

// --- Explicit permission requests ---
//
// `acquirePermissionsOnStartup` in init() does an initial probe but doesn't
// always force the underlying TCC registration call, which means the calling
// binary may not appear in System Settings → Privacy & Security. Calling
// `requestPermission()` explicitly for each required permission forces the
// SDK's native binary to make the OS-level call that registers it.
//
// macOS only shows the system dialog ONCE per (binary, permission) pair.
// After first denial the call is a silent no-op — but the binary is now
// in System Settings where the user can toggle it on. This is the same
// pattern Granola/Loom/etc. use.
for (const perm of REQUIRED_PERMISSIONS) {
  try {
    await RecallAiSdk.requestPermission(perm);
    emitLog("info", `requestPermission(${perm}) returned`);
  } catch (err) {
    // Best-effort — a failure here is logged and the next perm is tried.
    emitLog(
      "warn",
      `requestPermission(${perm}) failed: ${err?.message ?? err}`,
    );
  }
}

// --- Permission probe ---
//
// The SDK only emits `permission-status` for permissions whose state has
// actively changed since the last probe — granted permissions and ones the
// user has never touched stay silent. The Svelte UI needs to know about
// ALL five so it can show "Needed" rows. We emit a synthetic
// `status: "not-determined"` for any permission that hasn't reported by
// the time the probe window closes — real SDK events that arrive later
// will overwrite the synthetic value in the renderer's reactive store.

const probedPermissions = new Set();
const seenPermissionUpdate = (p) => probedPermissions.add(p);

RecallAiSdk.addEventListener("permission-status", (e) => {
  if (e?.permission) seenPermissionUpdate(e.permission);
});
RecallAiSdk.addEventListener("permissions-granted", () => {
  for (const p of REQUIRED_PERMISSIONS) seenPermissionUpdate(p);
});

setTimeout(() => {
  for (const perm of REQUIRED_PERMISSIONS) {
    if (!probedPermissions.has(perm)) {
      emitNdjson({
        type: "permission:status",
        permission: perm,
        status: "not-determined",
      });
      emitLog("info", `synthetic not-determined for ${perm}`);
    }
  }
}, 2500);

// --- Event wiring ---

RecallAiSdk.addEventListener("meeting-detected", (event) => {
  try {
    const window = event?.window ?? {};
    const rawUrl = typeof window.url === "string" ? window.url.trim() : "";
    const windowId = typeof window.id === "string" ? window.id : "";
    const platform = normalisePlatform(window.platform);

    // For unscheduled / single-window Zoom meetings the SDK often can't
    // extract a URL — the meeting was joined from the Zoom app directly
    // rather than from a calendar link, so there's nothing to scrape.
    // Forward the detection anyway: downstream dedup keys on this URL
    // string, and the notification logic can fall back to "Zoom meeting"
    // as the title without a real join URL. Dropping these used to
    // silently hide every unscheduled meeting on this machine.
    //
    // We keep `source: sdk-active-app` (the Rust DetectionSource enum
    // only knows `sdk-calendar` and `sdk-active-app` — adding a third
    // variant would be a wire change). For URL-less detections we encode
    // the windowId into a `recall-window:<id>` synthetic URL — both
    // recognisable in logs and stable for dedup.
    const fallbackKey = windowId ? `recall-window:${windowId}` : "";
    const meetingUrl = rawUrl || fallbackKey;
    if (!meetingUrl) {
      // No URL and no windowId — nothing we can dedup on. Drop with a log.
      emitLog("warn", "meeting-detected with no url and no windowId — dropping");
      return;
    }

    emitNdjson({
      type: "meeting:detected",
      detectionId: randomUUID(),
      meetingUrl,
      // Always include windowId — it's the canonical SDK handle and the
      // only stable identifier across `startRecording({ windowId })` and
      // `meeting-closed` events. For URL-having detections we encoded it
      // into the synthetic URL only as a fallback dedup key; downstream
      // callers should prefer `windowId` directly.
      windowId: windowId || undefined,
      platform,
      detectedAt: new Date().toISOString(),
      source: "sdk-active-app",
    });
    if (!rawUrl) {
      emitLog(
        "info",
        `meeting-detected forwarded without url (synthetic=${meetingUrl}, platform=${platform || "?"})`,
      );
    }
  } catch (err) {
    emitLog("error", `event translate failed: ${err?.message ?? err}`);
  }
});

RecallAiSdk.addEventListener("error", (event) => {
  emitLog("warn", `sdk error: ${event?.type ?? "?"} — ${event?.message ?? ""}`);
});

RecallAiSdk.addEventListener("log", (event) => {
  // Forward SDK internal logs at info or higher to stderr so they land in
  // ~/.hq/sync-debug.log. Drop debug-level to avoid noise.
  if (event?.level && event.level !== "debug") {
    emitLog(event.level, `[sdk:${event.subsystem ?? "?"}] ${event.message ?? ""}`);
  }
});

RecallAiSdk.addEventListener("permission-status", (event) => {
  // Surface to Svelte via the ndjson protocol so the UI can show a precise
  // per-permission status row. recall_sdk.rs parses this and emits a typed
  // Tauri event `permission:status`.
  if (event?.permission && event?.status) {
    emitNdjson({
      type: "permission:status",
      permission: event.permission,
      status: event.status,
    });
  }
  emitLog("info", `permission ${event?.permission} status=${event?.status}`);
});

RecallAiSdk.addEventListener("permissions-granted", () => {
  // Mark all required permissions as granted (the event itself doesn't carry
  // the per-permission detail; the prior per-permission events already
  // covered status, so this is just a convenience signal for the UI to
  // collapse any "needs permissions" banner).
  emitNdjson({
    type: "permissions:all-granted",
  });
  emitLog("info", "all required permissions granted");
});

RecallAiSdk.addEventListener("meeting-closed", (event) => {
  // SDK fires this when the meeting window goes away (user quits Zoom,
  // tab closes, Slack huddle ends, etc.). Lets the UI clear the row from
  // the active-meetings list so stale rows don't pile up if the user
  // chose not to record.
  const window = event?.window ?? {};
  emitNdjson({
    type: "meeting:closed",
    windowId: typeof window.id === "string" ? window.id : "",
    platform: normalisePlatform(window.platform),
    closedAt: new Date().toISOString(),
  });
  emitLog(
    "info",
    `meeting-closed (windowId=${window.id ?? "?"}, platform=${window.platform ?? "?"})`,
  );
});

RecallAiSdk.addEventListener("shutdown", (event) => {
  emitLog("info", `SDK shutdown (code=${event?.code} signal=${event?.signal})`);
});

// --- Recording lifecycle events ---
//
// The SDK fires these in response to startRecording/stopRecording calls
// (and to meeting-app state changes — e.g. a meeting closing auto-ends
// the recording). The shape we forward to Rust mirrors the
// `meeting:detected` convention: discriminator under `type`, windowId
// surfaced separately so the Svelte side can key on it directly.

RecallAiSdk.addEventListener("recording-started", (event) => {
  const window = event?.window ?? {};
  emitNdjson({
    type: "recording:started",
    windowId: typeof window.id === "string" ? window.id : "",
    platform: normalisePlatform(window.platform),
    startedAt: new Date().toISOString(),
  });
  emitLog(
    "info",
    `recording-started (windowId=${window.id ?? "?"}, platform=${window.platform ?? "?"})`,
  );
});

RecallAiSdk.addEventListener("recording-ended", (event) => {
  const window = event?.window ?? {};
  emitNdjson({
    type: "recording:ended",
    windowId: typeof window.id === "string" ? window.id : "",
    platform: normalisePlatform(window.platform),
    endedAt: new Date().toISOString(),
  });
  emitLog(
    "info",
    `recording-ended (windowId=${window.id ?? "?"}, platform=${window.platform ?? "?"})`,
  );
});

RecallAiSdk.addEventListener("media-capture-status", (event) => {
  // Tells us audio/video capture is actively running for a window. Useful
  // for the UI: "Recording…" indicator only flips on once we see at least
  // one capturing=true event (a recording-started without media capture
  // means the SDK accepted the request but hasn't latched onto a source
  // yet — possibly because of a permission glitch).
  emitNdjson({
    type: "recording:media-capture",
    windowId: event?.window?.id ?? "",
    captureType: event?.type ?? "",
    capturing: Boolean(event?.capturing),
  });
});

// --- Command channel (stdin → SDK) ---
//
// Rust's recall_sdk.rs writes JSON-per-line commands to our stdin to
// drive recording start/stop without spawning a new SDK process per
// recording. Wire format:
//
//   {"cmd":"start-recording","windowId":"<uuid>","uploadToken":"<token>"}
//   {"cmd":"stop-recording","windowId":"<uuid>"}
//   {"cmd":"pause-recording","windowId":"<uuid>"}
//   {"cmd":"resume-recording","windowId":"<uuid>"}
//
// Unknown commands are logged and skipped (forward-compat). Malformed
// JSON lines are logged at warn-level — the line is discarded but the
// loop keeps running so a single bad write doesn't kill the SDK process.

async function handleCommand(cmd) {
  const windowId = typeof cmd?.windowId === "string" ? cmd.windowId : "";
  if (!windowId) {
    emitLog("warn", `command missing windowId: ${JSON.stringify(cmd)}`);
    return;
  }

  try {
    switch (cmd.cmd) {
      case "start-recording": {
        const uploadToken =
          typeof cmd.uploadToken === "string" ? cmd.uploadToken : "";
        if (!uploadToken) {
          emitLog(
            "warn",
            `start-recording missing uploadToken (windowId=${windowId})`,
          );
          return;
        }
        emitLog("info", `start-recording: windowId=${windowId}`);
        await RecallAiSdk.startRecording({ windowId, uploadToken });
        return;
      }
      case "stop-recording": {
        emitLog("info", `stop-recording: windowId=${windowId}`);
        await RecallAiSdk.stopRecording({ windowId });
        return;
      }
      case "pause-recording": {
        emitLog("info", `pause-recording: windowId=${windowId}`);
        await RecallAiSdk.pauseRecording({ windowId });
        return;
      }
      case "resume-recording": {
        emitLog("info", `resume-recording: windowId=${windowId}`);
        await RecallAiSdk.resumeRecording({ windowId });
        return;
      }
      default:
        emitLog("warn", `unknown command: ${cmd.cmd}`);
    }
  } catch (err) {
    emitLog(
      "error",
      `command ${cmd.cmd} (windowId=${windowId}) failed: ${err?.message ?? err}`,
    );
    // Surface failures back to Rust as a typed error event so the UI
    // can show "couldn't start recording" instead of just spinning.
    emitNdjson({
      type: "recording:error",
      cmd: cmd.cmd,
      windowId,
      message: String(err?.message ?? err),
    });
  }
}

const stdinReader = createInterface({ input: process.stdin });
stdinReader.on("line", (line) => {
  const trimmed = line.trim();
  if (!trimmed) return;
  let parsed;
  try {
    parsed = JSON.parse(trimmed);
  } catch (err) {
    emitLog(
      "warn",
      `command parse failed (${err?.message ?? err}): ${trimmed.slice(0, 120)}`,
    );
    return;
  }
  handleCommand(parsed).catch((err) => {
    emitLog("error", `handleCommand crashed: ${err?.message ?? err}`);
  });
});
stdinReader.on("close", () => {
  // Rust closed our stdin — usually means the parent is shutting us
  // down. Initiate graceful shutdown so we don't linger.
  emitLog("info", "stdin closed; initiating graceful shutdown");
  gracefulShutdown("stdin-close");
});

// --- Signal handling ---

let shuttingDown = false;
async function gracefulShutdown(signal) {
  if (shuttingDown) return;
  shuttingDown = true;
  emitLog("info", `received ${signal}, shutting down`);
  try {
    await Promise.race([
      RecallAiSdk.shutdown(),
      new Promise((resolve) => setTimeout(resolve, 3500)),
    ]);
  } catch (err) {
    emitLog("warn", `shutdown error (continuing): ${err?.message ?? err}`);
  }
  process.exit(0);
}

process.on("SIGTERM", () => gracefulShutdown("SIGTERM"));
process.on("SIGINT", () => gracefulShutdown("SIGINT"));

// Keep the event loop alive until SIGTERM. The SDK holds open handles
// internally so we don't need an explicit no-op interval, but add one as
// a belt-and-suspenders to avoid any Node.js exits-when-idle surprise.
setInterval(() => {}, 60_000);
