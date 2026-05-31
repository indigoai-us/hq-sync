import { spawn, type ChildProcess } from 'node:child_process';
import {
  commandOnPath,
  DesktopAltHarness,
  reportDriverMode,
  type DesktopAltSnapshot,
  type DesktopAltTestHarness,
  type DesktopAltWindowState,
  type RenderedPage,
} from './harness';

type DesktopRouteName = 'sync' | 'meetings' | 'company';

interface LiveConfig {
  appPath: string;
  webdriverUrl: string;
}

interface DriverStart {
  client: WebDriverClient;
  process: ChildProcess | null;
}

interface WebDriverResponse<T> {
  value?: T;
  sessionId?: string;
}

interface SessionValue {
  sessionId?: string;
  capabilities?: Record<string, unknown>;
}

interface WebDriverLogEntry {
  level?: string;
  message?: string;
}

const DESKTOP_ALT_SELECTOR = '#desktop-alt, html[data-window="desktop-alt"]';
const POPOVER_TOGGLE_SELECTOR = '[data-testid="desktop-alt-toggle"]';
const ERROR_CAPTURE_SCRIPT = `
  if (!window.__desktopAltE2eErrors) {
    window.__desktopAltE2eErrors = [];
    const pushError = (value) => window.__desktopAltE2eErrors.push(String(value));
    const originalConsoleError = console.error.bind(console);
    console.error = (...args) => {
      pushError(args.map((arg) => {
        if (arg instanceof Error) return arg.stack || arg.message;
        if (typeof arg === 'string') return arg;
        try { return JSON.stringify(arg); } catch (_) { return String(arg); }
      }).join(' '));
      originalConsoleError(...args);
    };
    window.addEventListener('error', (event) => {
      pushError(event.error?.stack || event.message || 'window error');
    });
    window.addEventListener('unhandledrejection', (event) => {
      const reason = event.reason;
      pushError(reason?.stack || reason?.message || reason || 'unhandled rejection');
    });
  }
  return true;
`;

export async function createDesktopAltHarness(email: string): Promise<DesktopAltTestHarness> {
  const resolution = await resolveLiveConfig();

  if (!resolution.config) {
    reportDriverMode(resolution.reason);
    return new DesktopAltHarness(email);
  }

  const start = await startOrReuseDriver(resolution.config);

  try {
    const live = await LiveDesktopAltHarness.create(start.client, start.process);
    console.log(
      `[desktop-alt-e2e] live tauri-driver harness active at ${resolution.config.webdriverUrl}.`,
    );
    return live;
  } catch (error) {
    start.process?.kill();
    throw error;
  }
}

class LiveDesktopAltHarness implements DesktopAltTestHarness {
  readonly mode = 'live';

  private constructor(
    private readonly driver: WebDriverClient,
    private readonly driverProcess: ChildProcess | null,
  ) {}

  static async create(
    driver: WebDriverClient,
    driverProcess: ChildProcess | null,
  ): Promise<LiveDesktopAltHarness> {
    await driver.waitForWindow();
    const harness = new LiveDesktopAltHarness(driver, driverProcess);
    await harness.installErrorCaptureForAllWindows();
    return harness;
  }

  async bootPopover(): Promise<{ toggleVisible: boolean }> {
    await this.installErrorCaptureForAllWindows();
    const popover = await this.findWindowWithSelector(POPOVER_TOGGLE_SELECTOR);
    if (popover) await this.driver.switchToWindow(popover);
    return { toggleVisible: Boolean(popover) };
  }

  async clickDesktopAltToggle(): Promise<DesktopAltWindowState> {
    const existingDesktop = await this.findDesktopAltWindow();

    await this.openDesktopAltWindow();

    const desktop = await this.waitForDesktopAltWindow();
    await this.driver.switchToWindow(desktop);
    await this.installErrorCapture();

    return {
      id: desktop,
      focused: true,
      created: existingDesktop !== desktop,
    };
  }

  async closeDesktopAltWindow(): Promise<void> {
    const desktop = await this.findDesktopAltWindow();
    if (!desktop) return;

    await this.driver.switchToWindow(desktop);
    await this.driver.closeCurrentWindow();
    await this.driver.waitUntil(async () => !(await this.findDesktopAltWindow()), 5_000);

    const [remainingWindow] = await this.driver.getWindowHandles();
    if (remainingWindow) await this.driver.switchToWindow(remainingWindow);
  }

  async snapshot(): Promise<DesktopAltSnapshot> {
    const desktop = await this.findDesktopAltWindow();
    const popoverAlive = Boolean(await this.findResponsiveNonDesktopWindow(desktop));
    const trayAlive = await this.invokeTauriCommand('set_tray_state', { state: 'idle' });

    return {
      popoverAlive,
      trayAlive,
      desktopAltWindow: desktop ? { id: desktop, focused: true } : null,
    };
  }

  async navigate(route: DesktopRouteName): Promise<RenderedPage> {
    const desktop = await this.waitForDesktopAltWindow();
    await this.driver.switchToWindow(desktop);
    await this.installErrorCapture();

    if (route === 'company') {
      const clicked = await this.driver.execute<boolean>(`
        const button = document.querySelector('nav[aria-label="Companies"] button');
        if (!button) return false;
        button.click();
        return true;
      `);
      if (!clicked) {
        throw new Error('Live desktop-alt company navigation requires at least one company row.');
      }
      await this.waitForText('Companies');
    } else {
      await this.clickButtonWithText(route === 'sync' ? 'Sync' : 'Meetings');
      await this.waitForText(route === 'sync' ? 'Recent activity' : 'Connected calendars');
    }

    const text = await this.visibleText();
    return {
      route,
      text,
      consoleErrors: await this.collectConsoleErrors(),
    };
  }

  async dispose(): Promise<void> {
    await this.driver.deleteSession().catch(() => undefined);
    this.driverProcess?.kill();
  }

  private async openDesktopAltWindow(): Promise<void> {
    const popover = await this.findWindowWithSelector(POPOVER_TOGGLE_SELECTOR);
    if (popover) {
      await this.driver.switchToWindow(popover);
      const toggle = await this.driver.findElement(POPOVER_TOGGLE_SELECTOR);
      await this.driver.clickElement(toggle);
      return;
    }

    await this.invokeTauriCommand('open_desktop_alt_window', {});
  }

  private async clickButtonWithText(label: string): Promise<void> {
    const clicked = await this.driver.execute<boolean>(
      `
        const label = arguments[0];
        const buttons = Array.from(document.querySelectorAll('button'));
        const button = buttons.find((candidate) => candidate.textContent?.trim() === label);
        if (!button) return false;
        button.click();
        return true;
      `,
      [label],
    );

    if (!clicked) throw new Error(`Could not find live desktop-alt navigation button: ${label}`);
  }

  private async waitForText(text: string): Promise<void> {
    await this.driver.waitUntil(async () => {
      const bodyText = await this.driver
        .execute<string>('return document.body?.innerText || "";')
        .catch(() => '');
      return bodyText.includes(text);
    }, 5_000);
  }

  private async visibleText(): Promise<string[]> {
    const bodyText = await this.driver.execute<string>('return document.body?.innerText || "";');
    return bodyText
      .split('\n')
      .map((line) => line.trim())
      .filter(Boolean);
  }

  private async collectConsoleErrors(): Promise<string[]> {
    const pageErrors = await this.driver
      .execute<unknown[]>('return window.__desktopAltE2eErrors || [];')
      .catch(() => []);
    const browserLogs = await this.driver.browserLogs().catch(() => []);
    const logErrors = browserLogs
      .filter((entry) => !entry.level || /error|severe/i.test(entry.level))
      .map((entry) => entry.message)
      .filter((message): message is string => Boolean(message));

    return [...pageErrors.map(String), ...logErrors];
  }

  private async installErrorCaptureForAllWindows(): Promise<void> {
    const handles = await this.driver.getWindowHandles();
    for (const handle of handles) {
      await this.driver.switchToWindow(handle).catch(() => undefined);
      await this.installErrorCapture().catch(() => undefined);
    }
  }

  private async installErrorCapture(): Promise<void> {
    await this.driver.execute<boolean>(ERROR_CAPTURE_SCRIPT);
  }

  private async findDesktopAltWindow(): Promise<string | null> {
    return this.findWindowWithPredicate(
      `
        return Boolean(
          document.querySelector(arguments[0]) ||
          location.href.includes('desktop-alt.html') ||
          document.documentElement?.dataset?.window === 'desktop-alt'
        );
      `,
      [DESKTOP_ALT_SELECTOR],
    );
  }

  private async waitForDesktopAltWindow(): Promise<string> {
    let desktop: string | null = null;
    await this.driver.waitUntil(async () => {
      desktop = await this.findDesktopAltWindow();
      return Boolean(desktop);
    }, 8_000);
    if (!desktop) throw new Error('Timed out waiting for the desktop-alt window.');
    return desktop;
  }

  private async findWindowWithSelector(selector: string): Promise<string | null> {
    return this.findWindowWithPredicate('return Boolean(document.querySelector(arguments[0]));', [
      selector,
    ]);
  }

  private async findResponsiveNonDesktopWindow(desktop: string | null): Promise<string | null> {
    const handles = await this.driver.getWindowHandles();
    for (const handle of handles) {
      if (handle === desktop) continue;
      const responsive = await this.driver
        .switchToWindow(handle)
        .then(() => this.driver.execute<boolean>('return document.readyState !== "loading";'))
        .catch(() => false);
      if (responsive) return handle;
    }
    return null;
  }

  private async findWindowWithPredicate(script: string, args: unknown[] = []): Promise<string | null> {
    const handles = await this.driver.getWindowHandles();
    for (const handle of handles) {
      const matches = await this.driver
        .switchToWindow(handle)
        .then(() => this.driver.execute<boolean>(script, args))
        .catch(() => false);
      if (matches) return handle;
    }
    return null;
  }

  private async invokeTauriCommand(command: string, args: Record<string, unknown>): Promise<boolean> {
    const result = await this.driver.executeAsync<{ ok: boolean; error?: string }>(
      `
        const command = arguments[0];
        const payload = arguments[1];
        const done = arguments[arguments.length - 1];
        const invoke = window.__TAURI__?.core?.invoke || window.__TAURI_INTERNALS__?.invoke;
        if (!invoke) {
          done({ ok: false, error: 'Tauri invoke bridge is not exposed to WebDriver.' });
          return;
        }
        Promise.resolve(invoke(command, payload))
          .then(() => done({ ok: true }))
          .catch((error) => done({ ok: false, error: String(error?.message || error) }));
      `,
      [command, args],
    );

    if (!result.ok) throw new Error(result.error ?? `Tauri command failed: ${command}`);
    return true;
  }
}

class WebDriverClient {
  private sessionId: string | null = null;

  constructor(private readonly baseUrl: string) {}

  async status(): Promise<boolean> {
    await this.raw('GET', '/status');
    return true;
  }

  async createSession(appPath: string): Promise<void> {
    const response = await this.raw<SessionValue>('POST', '/session', {
      capabilities: {
        alwaysMatch: {
          browserName: 'wry',
          'tauri:options': {
            application: appPath,
          },
        },
      },
    });

    this.sessionId = response.value?.sessionId ?? response.sessionId ?? null;
    if (!this.sessionId) throw new Error('tauri-driver did not return a WebDriver session id.');
  }

  async waitForWindow(): Promise<void> {
    await this.waitUntil(async () => (await this.getWindowHandles()).length > 0, 10_000);
  }

  async getWindowHandles(): Promise<string[]> {
    return this.send<string[]>('GET', '/window/handles');
  }

  async switchToWindow(handle: string): Promise<void> {
    await this.send<null>('POST', '/window', { handle });
  }

  async closeCurrentWindow(): Promise<void> {
    await this.send<null>('DELETE', '/window');
  }

  async findElement(selector: string): Promise<string> {
    const element = await this.send<Record<string, string>>('POST', '/element', {
      using: 'css selector',
      value: selector,
    });
    const elementId = element['element-6066-11e4-a52e-4f735466cecf'] ?? element.ELEMENT;
    if (!elementId) throw new Error(`WebDriver did not return an element id for ${selector}.`);
    return elementId;
  }

  async clickElement(elementId: string): Promise<void> {
    await this.send<null>('POST', `/element/${encodeURIComponent(elementId)}/click`);
  }

  async execute<T>(script: string, args: unknown[] = []): Promise<T> {
    return this.send<T>('POST', '/execute/sync', { script, args });
  }

  async executeAsync<T>(script: string, args: unknown[] = []): Promise<T> {
    return this.send<T>('POST', '/execute/async', { script, args });
  }

  async browserLogs(): Promise<WebDriverLogEntry[]> {
    return this.send<WebDriverLogEntry[]>('POST', '/log', { type: 'browser' });
  }

  async deleteSession(): Promise<void> {
    if (!this.sessionId) return;
    await this.raw('DELETE', `/session/${this.sessionId}`);
    this.sessionId = null;
  }

  async waitUntil(predicate: () => Promise<boolean>, timeoutMs: number): Promise<void> {
    const deadline = Date.now() + timeoutMs;
    let lastError: unknown = null;

    while (Date.now() < deadline) {
      try {
        if (await predicate()) return;
      } catch (error) {
        lastError = error;
      }
      await sleep(100);
    }

    throw new Error(
      lastError instanceof Error
        ? `Timed out waiting for WebDriver condition: ${lastError.message}`
        : 'Timed out waiting for WebDriver condition.',
    );
  }

  private async send<T>(method: string, path: string, body?: unknown): Promise<T> {
    if (!this.sessionId) throw new Error('WebDriver session has not been created.');
    const response = await this.raw<T>(method, `/session/${this.sessionId}${path}`, body);
    return response.value as T;
  }

  private async raw<T>(
    method: string,
    path: string,
    body?: unknown,
  ): Promise<WebDriverResponse<T>> {
    const response = await fetch(new URL(path, this.baseUrl), {
      method,
      headers: body === undefined ? undefined : { 'content-type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    const payload = (await response.json().catch(() => ({}))) as WebDriverResponse<T> & {
      value?: { message?: string };
    };

    if (!response.ok) {
      const message =
        typeof payload.value?.message === 'string'
          ? payload.value.message
          : `${method} ${path} failed with HTTP ${response.status}`;
      throw new Error(message);
    }

    return payload as WebDriverResponse<T>;
  }
}

async function resolveLiveConfig(): Promise<{ config: LiveConfig | null; reason: string }> {
  const appPath =
    process.env.HQ_SYNC_DESKTOP_ALT_APP ?? process.env.HQ_SYNC_DESKTOP_ALT_APP_PATH ?? '';
  const webdriverUrl = process.env.HQ_SYNC_DESKTOP_ALT_WEBDRIVER_URL ?? 'http://127.0.0.1:4444';
  const liveRequested = isTruthy(process.env.HQ_SYNC_DESKTOP_ALT_LIVE);

  if (!liveRequested && !appPath) {
    return {
      config: null,
      reason:
        'set HQ_SYNC_DESKTOP_ALT_LIVE=1 with HQ_SYNC_DESKTOP_ALT_APP to enable live tauri-driver checks',
    };
  }

  if (!appPath) {
    return {
      config: null,
      reason: 'HQ_SYNC_DESKTOP_ALT_LIVE was set but no HQ_SYNC_DESKTOP_ALT_APP path was provided',
    };
  }

  if (commandOnPath('tauri-driver')) {
    return { config: { appPath, webdriverUrl }, reason: '' };
  }

  const reusableClient = new WebDriverClient(webdriverUrl);
  const reusableDriverRunning = await reusableClient.status().catch(() => false);

  if (reusableDriverRunning) {
    return { config: { appPath, webdriverUrl }, reason: '' };
  }

  return {
    config: null,
    reason: 'live inputs were provided, but tauri-driver was not on PATH and no WebDriver server responded',
  };
}

async function startOrReuseDriver(config: LiveConfig): Promise<DriverStart> {
  const client = new WebDriverClient(config.webdriverUrl);
  const driverRunning = await client.status().catch(() => false);

  if (driverRunning) {
    await client.createSession(config.appPath);
    return { client, process: null };
  }

  const driverProcess = spawn(
    'tauri-driver',
    ['--port', String(new URL(config.webdriverUrl).port || 4444)],
    {
      env: { ...process.env, TAURI_WEBVIEW_AUTOMATION: 'true' },
      stdio: 'ignore',
    },
  );

  try {
    await client.waitUntil(() => client.status().catch(() => false), 10_000);
    await client.createSession(config.appPath);
    return { client, process: driverProcess };
  } catch (error) {
    driverProcess.kill();
    throw error;
  }
}

function isTruthy(value: string | undefined): boolean {
  return value === '1' || value?.toLowerCase() === 'true' || value?.toLowerCase() === 'yes';
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
