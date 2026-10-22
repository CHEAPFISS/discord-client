// Voice panel: join, mute/deaf, leave. All user-facing text goes through i18n.

import { api, type Channel } from '../api';
import { el } from '../dom';
import type { VoiceStatusPayload } from '../events';
import { t } from '../i18n';
import { state } from '../state';
import { renderVoiceMembers } from './members';

interface VoiceText {
  key: string;
  vars?: Record<string, string | number>;
}

let lastVoiceText: VoiceText | null = null;

/** Sets the voice panel status text and remembers the key for language switches. */
export function setVoiceStatusKey(key: string, vars?: Record<string, string | number>): void {
  lastVoiceText = { key, vars };
  el('voiceStatus').textContent = t(key, vars);
}

function clearVoiceStatus(): void {
  lastVoiceText = null;
  el('voiceStatus').textContent = '';
}

export async function joinVoice(channel: Channel): Promise<void> {
  const guildId = state.currentGuildId;
  if (!guildId) return;
  state.currentVoiceChannelId = channel.id;
  el('voiceChannelName').textContent = channel.name;
  setVoiceStatusKey('voice.connecting');
  el('voiceCall').classList.add('active');
  try {
    await api.joinVoice(channel.id, guildId);
  } catch (e) {
    setVoiceStatusKey('voice.error', { error: String(e) });
  }
}

export function paintVoiceButtons(): void {
  const mic = el('micBtn');
  mic.classList.toggle('active', state.micMuted);
  mic.title = state.micMuted ? t('voice.micMuted') : t('voice.mic');
  mic.textContent = state.micMuted ? '🔇' : '🎙';
  const deaf = el('deafBtn');
  deaf.classList.toggle('active', state.deafened);
  deaf.title = state.deafened ? t('voice.deafened') : t('voice.sound');
  deaf.textContent = state.deafened ? '🔇' : '🔊';
}

/** Re-applies the current language to dynamic voice-panel texts. */
export function refreshVoiceUi(): void {
  paintVoiceButtons();
  if (lastVoiceText) {
    el('voiceStatus').textContent = t(lastVoiceText.key, lastVoiceText.vars);
  }
}

/** Microphone/sound/hangup buttons. */
export function initVoiceControls(): void {
  el('micBtn').onclick = async () => {
    // Deafen implies mute: clicking the mic first lifts deafen.
    if (state.deafened) {
      state.deafened = false;
      state.micMuted = false;
      try {
        await api.setSelfDeaf(false);
      } catch {
        // not in voice — fine
      }
      paintVoiceButtons();
      return;
    }
    state.micMuted = !state.micMuted;
    try {
      await api.setSelfMute(state.micMuted);
    } catch {
      // not in voice — fine
    }
    paintVoiceButtons();
  };

  el('deafBtn').onclick = async () => {
    state.deafened = !state.deafened;
    try {
      await api.setSelfDeaf(state.deafened);
    } catch {
      // not in voice — fine
    }
    // Like Discord: deafen also mutes the microphone; un-deafen lifts both.
    state.micMuted = state.deafened;
    paintVoiceButtons();
  };

  el('hangupBtn').onclick = async () => {
    try {
      await api.leaveVoice();
    } catch {
      // already left — fine
    }
    state.currentVoiceChannelId = null;
    clearVoiceStatus();
    el('voiceCall').classList.remove('active');
  };
}

/** Reacts to voice_status events from the backend. */
export function handleVoiceStatus(p: VoiceStatusPayload): void {
  if (p.status === 'left') {
    state.currentVoiceChannelId = null;
    clearVoiceStatus();
    el('voiceCall').classList.remove('active');
    // Joining a channel always starts unmuted/un-deaf (op 4) — reset the buttons.
    state.micMuted = false;
    state.deafened = false;
    state.voiceSpeaking.clear();
    renderVoiceMembers();
    paintVoiceButtons();
  } else if (p.status === 'joining') {
    // Channel switch (join without 'left'): the backend resets mute/deaf on
    // join, so sync the buttons here instead of showing a stale mute state.
    state.micMuted = false;
    state.deafened = false;
    paintVoiceButtons();
    setVoiceStatusKey('voice.connecting');
  } else if (p.status === 'connected') {
    setVoiceStatusKey('voice.connected');
  } else if (p.status === 'reconnecting') {
    setVoiceStatusKey('voice.reconnecting');
  } else if (p.status === 'error') {
    setVoiceStatusKey('voice.errorStatus', { error: p.error || t('voice.unknownError') });
  }
}
