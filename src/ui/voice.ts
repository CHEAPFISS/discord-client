// Голосовая панель: подключение, mute/deaf, отключение.

import { api, type Channel } from '../api';
import { el } from '../dom';
import type { VoiceStatusPayload } from '../events';
import { state } from '../state';

export function setVoiceStatus(text: string): void {
  el('voiceStatus').textContent = text;
}

export async function joinVoice(channel: Channel): Promise<void> {
  const guildId = state.currentGuildId;
  if (!guildId) return;
  state.currentVoiceChannelId = channel.id;
  el('voiceChannelName').textContent = channel.name;
  setVoiceStatus('Подключаюсь…');
  el('voiceCall').classList.add('active');
  try {
    await api.joinVoice(channel.id, guildId);
  } catch (e) {
    setVoiceStatus('Ошибка: ' + e);
  }
}

export function paintVoiceButtons(): void {
  const mic = el('micBtn');
  mic.classList.toggle('active', state.micMuted);
  mic.title = state.micMuted ? 'Микрофон заглушён' : 'Микрофон';
  mic.textContent = state.micMuted ? '🔇' : '🎙';
  const deaf = el('deafBtn');
  deaf.classList.toggle('active', state.deafened);
  deaf.title = state.deafened ? 'Звук отключён' : 'Звук';
  deaf.textContent = state.deafened ? '🔇' : '🔊';
}

/** Кнопки микрофона/звука/отключения. */
export function initVoiceControls(): void {
  el('micBtn').onclick = async () => {
    // Deafen подразумевает mute: клик по микрофону сначала снимает глухоту.
    if (state.deafened) {
      state.deafened = false;
      state.micMuted = false;
      try {
        await api.setSelfDeaf(false);
      } catch {
        // не в голосе — не страшно
      }
      paintVoiceButtons();
      return;
    }
    state.micMuted = !state.micMuted;
    try {
      await api.setSelfMute(state.micMuted);
    } catch {
      // не в голосе — не страшно
    }
    paintVoiceButtons();
  };

  el('deafBtn').onclick = async () => {
    state.deafened = !state.deafened;
    try {
      await api.setSelfDeaf(state.deafened);
    } catch {
      // не в голосе — не страшно
    }
    // Как в Discord: deafen тянет мьют микрофона, un-deaf снимает оба.
    state.micMuted = state.deafened;
    paintVoiceButtons();
  };

  el('hangupBtn').onclick = async () => {
    try {
      await api.leaveVoice();
    } catch {
      // уже вышли — не страшно
    }
    state.currentVoiceChannelId = null;
    setVoiceStatus('');
    el('voiceCall').classList.remove('active');
  };
}

/** Реакция UI на события voice_status от бэкенда. */
export function handleVoiceStatus(p: VoiceStatusPayload): void {
  if (p.status === 'left') {
    state.currentVoiceChannelId = null;
    setVoiceStatus('');
    el('voiceCall').classList.remove('active');
    // Свежий вход в канал всегда unmuted/un-deaf (op 4 при join) — кнопки сбросим.
    state.micMuted = false;
    state.deafened = false;
    paintVoiceButtons();
  } else if (p.status === 'joining') {
    setVoiceStatus('Подключаюсь…');
  } else if (p.status === 'connected') {
    setVoiceStatus('Голос: подключено (DAVE/E2EE) 🎧');
  } else if (p.status === 'reconnecting') {
    setVoiceStatus('Голос: переподключение…');
  } else if (p.status === 'error') {
    setVoiceStatus('Ошибка голоса: ' + (p.error || 'неизвестно'));
  }
}
