// Бейджи непрочитанных сообщений.

import { state } from '../state';

export function updateBadge(channelId: string): void {
  const channelEl = state.channelEls[channelId];
  if (!channelEl) return;
  const badge = channelEl.querySelector<HTMLElement>('.unread-badge');
  if (!badge) return;
  const n = state.unread.get(channelId) || 0;
  if (n > 0) {
    badge.textContent = n > 9 ? '9+' : String(n);
    badge.classList.add('visible');
  } else {
    badge.classList.remove('visible');
  }
}
