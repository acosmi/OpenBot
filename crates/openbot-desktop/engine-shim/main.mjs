// Clean-room Electron edge for the Rust-owned engine protocol (v4 §11.2/§11.3).
import { app, BrowserWindow, protocol, session } from "electron";
import net from "node:net";
import { Buffer } from "node:buffer";
import process from "node:process";

import { FRAME_FIXED_HEADER_BYTES, FRAME_HELLO_MAGIC, FRAME_MAGIC, MAX_BOOT_BYTES, MAX_CONTROL_FRAME_BYTES, MAX_IMAGE_BYTES, PROTOCOL, PROTOCOL_VERSION, RELEASE_EPOCH } from "./generated/protocol.mjs";

const BROWSER_SCHEME = "acosmi-engine";
const COMPONENT_SCHEME = "component";
const VIEWPORT_WIDTH = 1280;
const VIEWPORT_HEIGHT = 800;
const JPEG_QUALITY = 70;

app.enableSandbox();
protocol.registerSchemesAsPrivileged([
  {
    scheme: BROWSER_SCHEME,
    privileges: { standard: true, secure: true, supportFetchAPI: false, corsEnabled: false },
  },
  {
    scheme: COMPONENT_SCHEME,
    privileges: { standard: true, secure: true, supportFetchAPI: false, corsEnabled: false },
  },
]);

let boot = null;
let control = null;
let frame = null;
let engineSession = null;
let active = null;
let frameSequence = 0n;
let inputBuffer = Buffer.alloc(0);
let commandChain = Promise.resolve();
let fatalStarted = false;

void bootstrap().catch((error) => fatal(error instanceof EngineFailure ? error.code : "bootstrap_failed"));

async function bootstrap() {
  boot = await readBootCapability();
  control = await connectPipe(boot.control_pipe);
  frame = await connectPipe(boot.frame_pipe);
  frame.write(
    Buffer.concat([
      Buffer.from(FRAME_HELLO_MAGIC, "ascii"),
      Buffer.from(boot.token, "hex"),
    ]),
  );
  sendControl(control, { kind: "hello", token: boot.token });
  await app.whenReady();
  engineSession = session.fromPartition(partitionFor(boot));
  await configureSession(engineSession, boot.role);
  const mainMetric = app.getAppMetrics().find((entry) => entry.pid === process.pid);
  if (!(mainMetric?.creationTime > 0)) {
    throw new EngineFailure("main_metric_missing");
  }
  installPipeHandlers();
  sendControl(control, {
    kind: "ready",
    main_pid: process.pid,
    main_creation_time: mainMetric.creationTime,
    protocol_version: PROTOCOL_VERSION,
  });
}

function installPipeHandlers() {
  control.on("data", (chunk) => {
    inputBuffer = Buffer.concat([inputBuffer, chunk]);
    if (inputBuffer.length > MAX_CONTROL_FRAME_BYTES) {
      fatal("control_frame_too_large");
      return;
    }
    while (true) {
      const newline = inputBuffer.indexOf(0x0a);
      if (newline < 0) return;
      const line = inputBuffer.subarray(0, newline);
      inputBuffer = inputBuffer.subarray(newline + 1);
      if (line.length === 0) continue;
      let command;
      try {
        command = parseControlLine(line);
      } catch (_error) {
        fatal("control_frame_invalid");
        return;
      }
      commandChain = commandChain
        .then(() => handleCommand(command))
        .catch((error) => reportCommandError(command.operation_id, error));
    }
  });
  control.on("close", () => {
    void shutdownEngine(false);
  });
  control.on("error", () => fatal("control_pipe_failed"));
  frame.on("error", () => fatal("frame_pipe_failed"));
}

async function readBootCapability() {
  let bytes = Buffer.alloc(0);
  for await (const chunk of process.stdin) {
    bytes = Buffer.concat([bytes, chunk]);
    if (bytes.length > MAX_BOOT_BYTES) throw new Error("boot_too_large");
    const newline = bytes.indexOf(0x0a);
    if (newline < 0) continue;
    const trailing = bytes.subarray(newline + 1).toString("utf8").trim();
    if (trailing.length !== 0) throw new Error("boot_trailing_data");
    const parsed = JSON.parse(bytes.subarray(0, newline).toString("utf8"));
    validateBoot(parsed);
    return parsed;
  }
  throw new Error("boot_missing");
}

function validateBoot(value) {
  requireExactKeys(value, [
    "computer_id",
    "control_pipe",
    "frame_pipe",
    "generation",
    "protocol_version",
    "release_epoch",
    "role",
    "scope_digest",
    "token",
  ]);
  if (value.protocol_version !== PROTOCOL_VERSION) throw new Error("protocol_mismatch");
  if (value.release_epoch !== String(RELEASE_EPOCH)) throw new Error("release_epoch_mismatch");
  if (!PROTOCOL.roles.includes(value.role)) throw new Error("role_invalid");
  if (!isBoundedString(value.control_pipe, 512) || !isBoundedString(value.frame_pipe, 512)) {
    throw new Error("pipe_invalid");
  }
  if (!isBoundedString(value.computer_id, 256)) throw new Error("computer_invalid");
  if (!/^[0-9]+$/.test(value.generation)) throw new Error("generation_invalid");
  if (!/^[0-9a-f]{64}$/.test(value.scope_digest)) throw new Error("scope_invalid");
  if (!/^[0-9a-f]{32}$/.test(value.token)) throw new Error("token_invalid");
}

function connectPipe(path) {
  return new Promise((resolve, reject) => {
    const socket = net.createConnection({ path });
    socket.once("connect", () => resolve(socket));
    socket.once("error", reject);
  });
}

function partitionFor(value) {
  if (value.role === "browser_computer") {
    return `persist:ob-${value.scope_digest}`;
  }
  return `ob-component-${value.token}`;
}

async function configureSession(engineSession, role) {
  await engineSession.setProxy({
    mode: "fixed_servers",
    proxyRules: "http=127.0.0.1:1;https=127.0.0.1:1",
    proxyBypassRules: "<-loopback>",
  });
  engineSession.setPermissionRequestHandler((_contents, _permission, callback) => callback(false));
  engineSession.setPermissionCheckHandler(() => false);
  engineSession.webRequest.onBeforeRequest((details, callback) => {
    const parsed = new URL(details.url);
    const allowedScheme = role === "browser_computer" ? BROWSER_SCHEME : COMPONENT_SCHEME;
    const allowed =
      parsed.protocol === `${allowedScheme}:` ||
      (role === "sandboxed_component" && ["data:", "blob:"].includes(parsed.protocol));
    callback({ cancel: !allowed });
  });
  const scheme = role === "browser_computer" ? BROWSER_SCHEME : COMPONENT_SCHEME;
  engineSession.protocol.handle(scheme, (request) => internalDocument(request, scheme, role));
}

function internalDocument(request, scheme, role) {
  const parsed = new URL(request.url);
  if (parsed.protocol !== `${scheme}:` || parsed.hostname !== "session") {
    return new Response("", { status: 404 });
  }
  const label = role === "browser_computer" ? "Browser engine ready" : "Component engine ready";
  const html = `<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; connect-src 'none'; style-src 'unsafe-inline'; img-src data: blob:"><style>html,body{margin:0;width:100%;height:100%;background:#f4f4f2;color:#202020;font:16px system-ui}main{display:grid;place-items:center;width:100%;height:100%}</style></head><body><main>${label}</main></body></html>`;
  return new Response(html, {
    status: 200,
    headers: { "content-type": "text/html; charset=utf-8", "cache-control": "no-store" },
  });
}

function parseControlLine(line) {
  const value = JSON.parse(line.toString("utf8"));
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error("control_shape");
  }
  if (!PROTOCOL.commands.includes(value.kind)) throw new Error("control_kind");
  return value;
}

async function handleCommand(command) {
  if (command.kind === "start") {
    requireExactKeys(command, ["computer_id", "generation", "kind", "operation_id", "tab_id"]);
    validateScope(command);
    if (!isBoundedString(command.operation_id, 128) || !isBoundedString(command.tab_id, 256)) {
      throw new Error("start_invalid");
    }
    if (active !== null) {
      sendControl(control, { kind: "error", operation_id: command.operation_id, code: "session_busy" });
      return;
    }
    await startSession(command);
    return;
  }
  if (command.kind === "stop") {
    requireExactKeys(command, ["computer_id", "generation", "kind", "operation_id", "tab_id"]);
    validateScope(command);
    if (!isBoundedString(command.operation_id, 128) || !isBoundedString(command.tab_id, 256)) {
      throw new Error("stop_invalid");
    }
    if (active === null || active.tabId !== command.tab_id) {
      sendControl(control, { kind: "error", operation_id: command.operation_id, code: "session_stale" });
      return;
    }
    await stopSession(command.operation_id);
    return;
  }
  requireExactKeys(command, ["kind", "operation_id"]);
  if (!isBoundedString(command.operation_id, 128)) throw new Error("shutdown_invalid");
  await shutdownEngine(true, command.operation_id);
}

function validateScope(command) {
  if (command.computer_id !== boot.computer_id || command.generation !== boot.generation) {
    throw new Error("scope_stale");
  }
}

async function startSession(command) {
  const scheme = boot.role === "browser_computer" ? BROWSER_SCHEME : COMPONENT_SCHEME;
  const target = `${scheme}://session/${encodeURIComponent(command.tab_id)}`;
  const window = new BrowserWindow({
    show: false,
    width: VIEWPORT_WIDTH,
    height: VIEWPORT_HEIGHT,
    useContentSize: true,
    backgroundColor: "#f4f4f2",
    webPreferences: {
      nodeIntegration: false,
      contextIsolation: true,
      sandbox: true,
      webSecurity: true,
      webviewTag: false,
      devTools: false,
      session: engineSession,
    },
  });
  active = { window, tabId: command.tab_id, operationId: command.operation_id };
  const contents = window.webContents;
  contents.setWindowOpenHandler(() => ({ action: "deny" }));
  contents.on("will-navigate", (event, url) => {
    if (url !== target) event.preventDefault();
  });
  contents.on("will-attach-webview", (event) => event.preventDefault());
  contents.on("render-process-gone", (_event, details) => {
    if (active?.operationId === command.operation_id) {
      const reason = ["clean-exit", "abnormal-exit", "killed", "crashed", "oom", "launch-failed", "integrity-failure"].includes(details.reason)
        ? details.reason.replace("-", "_")
        : "unknown";
      sendControl(control, { kind: "error", operation_id: command.operation_id, code: `renderer_gone_${reason}` });
    }
  });
  await withDeadline(window.loadURL(target), "load_timeout");
  contents.debugger.attach("1.3");
  try {
    await withDeadline(contents.debugger.sendCommand("Page.enable"), "page_enable_timeout");
    const probe = await withDeadline(
      contents.debugger.sendCommand("Runtime.evaluate", {
        expression: "({processType:typeof process,requireType:typeof require,origin:location.origin})",
        returnByValue: true,
      }),
      "runtime_probe_timeout",
    );
    const value = probe.result?.value;
    if (value?.processType !== "undefined" || value?.requireType !== "undefined") {
      throw new Error("renderer_node_exposed");
    }
    const capture = await withDeadline(
      contents.debugger.sendCommand("Page.captureScreenshot", {
        format: "jpeg",
        quality: JPEG_QUALITY,
        fromSurface: true,
      }),
      "capture_timeout",
    );
    const image = Buffer.from(capture.data, "base64");
    await sendFrame(image, command.tab_id, VIEWPORT_WIDTH, VIEWPORT_HEIGHT);
    const rendererPid = contents.getOSProcessId();
    const metric = app.getAppMetrics().find((entry) => entry.pid === rendererPid);
    if (metric?.sandboxed !== true || !(metric.creationTime > 0)) {
      throw new EngineFailure("renderer_not_sandboxed");
    }
    sendControl(control, {
      kind: "started",
      operation_id: command.operation_id,
      tab_id: command.tab_id,
      renderer_pid: rendererPid,
      renderer_creation_time: metric.creationTime,
      renderer_sandboxed: true,
      node_exposed: false,
      origin: value.origin,
    });
  } finally {
    if (contents.debugger.isAttached()) contents.debugger.detach();
  }
}

async function sendFrame(image, tabId, width, height) {
  if (image.length === 0 || image.length > MAX_IMAGE_BYTES) throw new Error("frame_size");
  const computer = Buffer.from(boot.computer_id, "utf8");
  const tab = Buffer.from(tabId, "utf8");
  if (computer.length > 0xffff || tab.length > 0xffff) throw new Error("frame_id_size");
  const headerLength = FRAME_FIXED_HEADER_BYTES + computer.length + tab.length;
  const header = Buffer.alloc(FRAME_FIXED_HEADER_BYTES);
  header.write(FRAME_MAGIC, 0, "ascii");
  header.writeUInt16LE(PROTOCOL_VERSION, 8);
  header.writeUInt8(boot.role === "browser_computer" ? 0 : 1, 10);
  header.writeUInt8(1, 11);
  header.writeUInt32LE(headerLength, 12);
  header.writeUInt32LE(image.length, 16);
  header.writeBigUInt64LE(BigInt(boot.generation), 20);
  frameSequence += 1n;
  header.writeBigUInt64LE(frameSequence, 28);
  header.writeUInt32LE(width, 36);
  header.writeUInt32LE(height, 40);
  header.writeUInt16LE(computer.length, 44);
  header.writeUInt16LE(tab.length, 46);
  const bytes = Buffer.concat([header, computer, tab, image]);
  const accepted = frame.write(bytes);
  if (!accepted) {
    await new Promise((resolve) => frame.once("drain", resolve));
  }
}

async function stopSession(operationId) {
  const current = active;
  active = null;
  if (current !== null && !current.window.isDestroyed()) current.window.destroy();
  sendControl(control, { kind: "stopped", operation_id: operationId });
}

async function shutdownEngine(acknowledge, operationId = "connection-closed") {
  if (active !== null) {
    const current = active;
    active = null;
    if (!current.window.isDestroyed()) current.window.destroy();
  }
  if (acknowledge) sendControl(control, { kind: "shutdown_complete", operation_id: operationId });
  control.end();
  frame.end();
  app.quit();
}

function sendControl(socket, value) {
  socket.write(`${JSON.stringify(value)}\n`);
}

function requireExactKeys(value, expected) {
  const actual = Object.keys(value).sort();
  const wanted = [...expected].sort();
  if (actual.length !== wanted.length || actual.some((key, index) => key !== wanted[index])) {
    throw new Error("keys_invalid");
  }
}

function isBoundedString(value, max) {
  return typeof value === "string" && value.length > 0 && Buffer.byteLength(value, "utf8") <= max;
}

class EngineFailure extends Error {
  constructor(code) {
    super(code);
    this.code = code;
  }
}

function withDeadline(promise, code) {
  let timer;
  const deadline = new Promise((_resolve, reject) => {
    timer = setTimeout(() => reject(new EngineFailure(code)), 5000);
  });
  return Promise.race([promise, deadline]).finally(() => clearTimeout(timer));
}

function reportCommandError(operationId, error) {
  const code = error instanceof EngineFailure ? error.code : "command_failed";
  sendControl(control, { kind: "error", operation_id: operationId, code });
  const current = active;
  active = null;
  if (current !== null && !current.window.isDestroyed()) current.window.destroy();
}

function fatal(code) {
  if (fatalStarted) return;
  fatalStarted = true;
  const current = active;
  active = null;
  if (current !== null && !current.window.isDestroyed()) current.window.destroy();
  process.exitCode = 1;
  const exit = () => {
    if (frame !== null && !frame.destroyed) frame.end();
    app.exit(1);
  };
  if (control !== null && !control.destroyed) {
    control.write(`${JSON.stringify({ kind: "error", code })}\n`, exit);
    setTimeout(exit, 250);
  } else {
    exit();
  }
}
