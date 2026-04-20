// Console is SPA-only — everything loads client-side from the gateway. This
// avoids cross-origin concerns in dev (console on :7071, gateway on :7070)
// and SSR-the-gateway-is-down edge cases.
export const ssr = false;
export const prerender = false;
