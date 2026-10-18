// Список каналов выбранного сервера.

import { api, type Channel, type Guild } from '../api';
import { el } from '../dom';
import { state } from '../state';
import { openTextChannel } from './chat';
import { loadMembers } from './members';
import { showError } from './toast';
import { updateBadge } from './unread';
import { joinVoice } from './voice';

/** Голосовой канал? (2 — voice, 13 — stage). */
export function isVoice(c: Channel): boolean {
  return c.kind === 2 || c.kind === 13;
}

export async function loadChannels(guild: Guild): Promise<void> {
  const channelsEl = el('channels');
  state.currentGuildId = guild.id;
  channelsEl.innerHTML = '';
  const cat = document.createElement('div');
  cat.className = 'channel-category';
  cat.textContent = guild.name;
  channelsEl.appendChild(cat);

  // Участники грузятся параллельно с каналами.
  void loadMembers(guild.id);

  try {
    const channels = await api.getChannels(guild.id);
    state.channels = channels;
    channels.forEach((c) => {
      const div = document.createElement('div');
      div.className = `channel-item ${isVoice(c) ? 'voice' : 'text'}`;
      div.dataset.channelId = c.id;
      const nameSpan = document.createElement('span');
      nameSpan.className = 'channel-name';
      nameSpan.textContent = c.name;
      const badge = document.createElement('span');
      badge.className = 'unread-badge';
      div.appendChild(nameSpan);
      div.appendChild(badge);
      state.channelEls[c.id] = div;
      updateBadge(c.id);

      div.onclick = () => {
        document
          .querySelectorAll('.channel-item')
          .forEach((node) => node.classList.remove('active'));
        div.classList.add('active');
        // сбрасываем непрочитанное для этого канала
        state.unread.delete(c.id);
        updateBadge(c.id);
        if (isVoice(c)) void joinVoice(c);
        else void openTextChannel(c);
      };
      channelsEl.appendChild(div);
    });
  } catch (e) {
    showError('Не удалось загрузить каналы: ' + e);
  }
}
