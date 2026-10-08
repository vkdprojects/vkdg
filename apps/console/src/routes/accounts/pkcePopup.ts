/**
 * OAuth popup + callback capture for the PKCE login step.
 *
 * claude.ai only redirects to its own code page or to a loopback `/callback`.
 * On a loopback console the callback page captures the code automatically;
 * anywhere else the user copies the code from claude.ai's page and pastes it.
 */

const CHANNEL = 'vkdg_oauth_callback';

/** Loopback `/callback` URL to send as `redirect_uri`, or `null` when the code must be pasted. */
export function loopbackRedirect(): string | null {
  const { protocol, hostname, origin } = window.location;
  if (protocol !== 'http:' || (hostname !== 'localhost' && hostname !== '127.0.0.1')) return null;
  return `${origin}/callback`;
}

/** What the /callback page posted on the channel, already validated. */
export type CaptureResult = { code: string } | { error: string };

export interface PkceCapture {
  /**
   * Open the authorize popup. Returns `true` when the code will be captured
   * automatically; `false` when the user has to paste it (blocked popup, no
   * loopback callback, or no BroadcastChannel).
   */
  open(authorizeUrl: string, manual: boolean, onresult: (r: CaptureResult) => void): boolean;
  /** Close popup and channel. Safe to call repeatedly. */
  close(): void;
}

export function createPkceCapture(): PkceCapture {
  let popup: Window | null = null;
  let channel: BroadcastChannel | null = null;

  return {
    open(authorizeUrl, manual, onresult) {
      const w = 520, h = 700;
      const left = Math.round(window.screenX + (window.outerWidth - w) / 2);
      const top = Math.round(window.screenY + (window.outerHeight - h) / 2);
      popup = window.open(
        authorizeUrl,
        'vkdg_oauth',
        `width=${w},height=${h},left=${left},top=${top},toolbar=0,menubar=0,location=1`,
      );

      // Blocked popup, or a non-loopback console with no callback to capture: the user pastes the code.
      if (!popup || manual) return false;

      // The /callback page posts the code on this channel. It is the only capture
      // path: claude.ai serves a Cross-Origin-Opener-Policy that severs `window.opener`
      // and makes `popup.closed` read true while the popup is still open, so neither
      // postMessage nor popup polling can be relied on.
      try { channel?.close(); } catch { /* ignore */ }
      try {
        channel = new BroadcastChannel(CHANNEL);
        channel.onmessage = (ev) => {
          const data = ev.data as Record<string, string> | null;
          if (data?.source !== CHANNEL) return;
          if (data.type === 'oauth_error') onresult({ error: data.error_description || data.error || 'OAuth error' });
          else if (data.type === 'oauth_code' && data.code) onresult({ code: data.code });
        };
      } catch {
        return false;
      }
      return true;
    },
    close() {
      try { channel?.close(); } catch { /* ignore */ }
      channel = null;
      try { popup?.close(); } catch { /* ignore */ }
      popup = null;
    },
  };
}
