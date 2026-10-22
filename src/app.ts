// Frontend entry point: wires event handlers and restores the session.
//
// Structure:
//   api.ts       — typed wrappers around Rust commands
//   events.ts    — typed subscriptions to backend events
//   state.ts     — client-side state
//   i18n.ts      — RU/EN dictionaries + language switch
//   login.ts     — sign-in screen + automatic session restore
//   ui/*.ts      — rendering: servers, channels, chat, members, voice, status, toasts

import { api } from './api';
import { el } from './dom';
import {
  onGatewayStatus,
  onMessageCreate,
  onPresenceUpdate,
  onStreamUpdate,
  onVoiceChannelReady,
  onVoiceMemberUpdate,
  onVoiceSpeaking,
  onVoiceStatus,
} from './events';
import { initI18n, onLanguageChange } from './i18n';
import { initLogin, tryRestoreSession } from './login';
import { appendLiveMessage, initChatInput, refreshChatTexts } from './ui/chat';
import {
  handlePresenceUpdate,
  handleVoiceMemberUpdate,
  handleVoiceSpeaking,
  renderMembers,
  renderVoiceMembers,
} from './ui/members';
import { refreshServerTexts } from './ui/servers';
import { handleStreamUpdate, initStreamScreen, refreshStreamUi } from './ui/stream';
import { refreshGatewayStatus, setGatewayStatus } from './ui/status';
import { handleVoiceStatus, initVoiceControls, refreshVoiceUi, setVoiceStatusKey } from './ui/voice';
import { state } from './state';

// Must run before anything renders text: applies the stored/system language.
initI18n();

// --- Live Gateway events ---
onMessageCreate((p) => appendLiveMessage(p));
onGatewayStatus((p) => setGatewayStatus(p.status, p.error));
onVoiceStatus((p) => handleVoiceStatus(p));
onVoiceChannelReady(() => {
  setVoiceStatusKey('app.voiceReady');
});
onPresenceUpdate((p) => handlePresenceUpdate(p));
onVoiceMemberUpdate((p) => handleVoiceMemberUpdate(p));
onVoiceSpeaking((p) => handleVoiceSpeaking(p));
onStreamUpdate((p) => handleStreamUpdate(p));

// After a full page reload gateway_status events are already gone — ask the
// backend for the current state instead (Issue 1).
void api
  .getGatewayStatus()
  .then((p) => setGatewayStatus(p.status, p.error ?? undefined))
  .catch(() => {});

// --- Language switch: re-render dynamic widgets in place ---
onLanguageChange(() => {
  refreshGatewayStatus();
  refreshVoiceUi();
  refreshStreamUi();
  refreshChatTexts();
  refreshServerTexts();
  if (state.membersGuildId) {
    renderMembers();
    renderVoiceMembers();
  }
});

// --- Local handlers ---
initLogin();
initChatInput();
initVoiceControls();
initStreamScreen();

el('logoutBtn').onclick = async () => {
  try {
    await api.logout();
  } catch {
    // even if the backend failed — just reload the UI
  }
  location.reload();
};

// Restore a saved session on startup.
void tryRestoreSession();
