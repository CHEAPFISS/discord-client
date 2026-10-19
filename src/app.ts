// Точка входа фронтенда: собирает обработчики и запускает восстановление сессии.
//
// Структура:
//   api.ts       — типизированные вызовы Rust-команд
//   events.ts    — типизированные подписки на события бэкенда
//   state.ts     — клиентский стейт
//   login.ts     — экран входа + автовосстановление сессии
//   ui/*.ts      — рендер: серверы, каналы, чат, участники, голос, статус, уведомления

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
import { initLogin, tryRestoreSession } from './login';
import { appendLiveMessage, initChatInput } from './ui/chat';
import { handlePresenceUpdate, handleVoiceMemberUpdate, handleVoiceSpeaking } from './ui/members';
import { handleStreamUpdate, initStreamScreen } from './ui/stream';
import { setGatewayStatus } from './ui/status';
import { handleVoiceStatus, initVoiceControls, setVoiceStatus } from './ui/voice';

// --- Живые события от Gateway ---
onMessageCreate((p) => appendLiveMessage(p));
onGatewayStatus((p) => setGatewayStatus(p.status, p.error));
onVoiceStatus((p) => handleVoiceStatus(p));
onVoiceChannelReady(() => {
  setVoiceStatus('Голос: данные собраны, подключаюсь к голосовому серверу…');
});
onPresenceUpdate((p) => handlePresenceUpdate(p));
onVoiceMemberUpdate((p) => handleVoiceMemberUpdate(p));
onVoiceSpeaking((p) => handleVoiceSpeaking(p));
onStreamUpdate((p) => handleStreamUpdate(p));

// После полной перезагрузки страницы события gateway_status уже не придут —
// спрашиваем текущее состояние у бэкенда (Issue 1).
void api
  .getGatewayStatus()
  .then((p) => setGatewayStatus(p.status, p.error ?? undefined))
  .catch(() => {});

// --- Локальные обработчики ---
initLogin();
initChatInput();
initVoiceControls();
initStreamScreen();

el('logoutBtn').onclick = async () => {
  try {
    await api.logout();
  } catch {
    // даже если не получилось — просто перезагружаем UI
  }
  location.reload();
};

// Автовосстановление сессии при загрузке
void tryRestoreSession();
