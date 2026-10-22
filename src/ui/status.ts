// Gateway connection status pill.
// Colors live entirely in CSS (main.css); this module owns state + text.

import { el } from '../dom';
import { t } from '../i18n';

let lastStatus = 'disconnected';
let lastError: string | null = null;

export function setGatewayStatus(status: string, error?: string | null): void {
  lastStatus = status;
  lastError = error ?? null;
  renderGatewayStatus();
}

/** Re-renders the pill using the current language (called on language change). */
export function refreshGatewayStatus(): void {
  renderGatewayStatus();
}

function renderGatewayStatus(): void {
  const pill = el('appStatus');
  const text = el('statusText');
  switch (lastStatus) {
    case 'connected':
    case 'ready':
      pill.dataset.state = 'connected';
      text.textContent = t('status.connected');
      break;
    case 'reconnecting':
      pill.dataset.state = 'reconnecting';
      text.textContent = lastError
        ? t('status.reconnectingWithError', { error: lastError })
        : t('status.reconnecting');
      break;
    default:
      pill.dataset.state = 'disconnected';
      text.textContent = t('status.disconnected');
  }
}
