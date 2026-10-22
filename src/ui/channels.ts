// Список каналов выбранного сервера.

import { api, type Channel, type Guild } from '../api';
import { el } from '../dom';
import { t } from '../i18n';
import { state } from '../state';
import { openTextChannel } from './chat';
import { loadMembers, renderVoiceMembers } from './members';
import { showError } from './toast';
import { updateBadge } from './unread';
import { joinVoice } from './voice';

/** Голосовой канал? (2 — voice, 13 — stage). */
export function isVoice(c: Channel): boolean {
  return c.kind === 2 || c.kind === 13;
}

// Гонка переключения серверов: ответ для старого guild'а не должен
// затереть список уже открытого нового (см. seq-guard в openTextChannel).
let loadSeq = 0;

export async function loadChannels(guild: Guild): Promise<void> {
  const seq = ++loadSeq;
  const channelsEl = el('channels');
  state.currentGuildId = guild.id;
  state.voiceMemberEls = {};
  state.channelEls = {}; // старые узлы канала больше не в дереве — бейджи к ним не привязываем
  channelsEl.innerHTML = '';
  const cat = document.createElement('div');
  cat.className = 'channel-category';
  cat.textContent = guild.name;
  channelsEl.appendChild(cat);

  try {
    const channels = await api.getChannels(guild.id);
    if (seq !== loadSeq) return; // сервер уже переключили — этот ответ устарел
    state.channels = channels;
    // Категории (kind 4) — только разделители, их не показываем как каналы.
    const list = channels.filter((c) => c.kind !== 4);
    // Участников тянем после каналов: нужен id текстового канала для подписки.
    const textChannel = channels.find((c) => c.kind === 0);
    if (textChannel) void loadMembers(guild.id, textChannel.id);
    list.forEach((c) => {
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

      // Под голосовым каналом — мини-список тех, кто в нём сидит.
      if (isVoice(c)) {
        const subs = document.createElement('div');
        subs.className = 'voice-members';
        state.voiceMemberEls[c.id] = subs;
        channelsEl.appendChild(subs);
      }
    });
    renderVoiceMembers();
  } catch (e) {
    showError(t('channels.loadError', { error: String(e) }));
  }
}
