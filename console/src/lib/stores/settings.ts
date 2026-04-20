// Settings store — persists the gateway URL and caller identity to
// localStorage. We avoid the full reactive-store dance for this since Svelte
// 5 runes handle it fine with a small wrapper.

import { browser } from '$app/environment';

const KEY = 'cfg.console.settings';

export interface Settings {
  gatewayUrl: string;
  user: string;
}

const DEFAULT: Settings = {
  gatewayUrl:
    (typeof window !== 'undefined' &&
      (window as any).__SIGNET_GATEWAY_URL__) ??
    'http://localhost:7070',
  user: 'console'
};

function read(): Settings {
  if (!browser) return DEFAULT;
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return DEFAULT;
    const parsed = JSON.parse(raw);
    return {
      gatewayUrl: typeof parsed.gatewayUrl === 'string' ? parsed.gatewayUrl : DEFAULT.gatewayUrl,
      user: typeof parsed.user === 'string' ? parsed.user : DEFAULT.user
    };
  } catch {
    return DEFAULT;
  }
}

export function loadSettings(): Settings {
  return read();
}

export function saveSettings(next: Settings) {
  if (!browser) return;
  localStorage.setItem(KEY, JSON.stringify(next));
}
