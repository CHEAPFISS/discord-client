// Точка входа фронтенда: собирает обработчики и запускает восстановление сессии.
//
// Структура:
//   api.ts       — типизированные вызовы Rust-команд
//   events.ts    — типизированные подписки на события бэкенда
//   state.ts     — клиентский стейт
//   login.ts     — экран входа + автовосстановление сессии
//   ui/*.ts      — рендер: серверы, каналы, чат, голос, статус, уведомления

import { api } from './api';
import { el } from './dom';
import { onGatewayStatus, onMessageCreate, onVoiceChannelReady, onVoiceStatus } from './events';
import { initLogin, tryRestoreSession } from './login';
import { appendLiveMessage, initChatInput } from './ui/chat';
import { setGatewayStatus } from './ui/status';
import { handleVoiceStatus, initVoiceControls, setVoiceStatus } from './ui/voice';

// --- Живые события от Gateway ---
onMessageCreate((p) => appendLiveMessage(p));
onGatewayStatus((p) => setGatewayStatus(p.status, p.error));
onVoiceStatus((p) => handleVoiceStatus(p));
onVoiceChannelReady(() => {
  setVoiceStatus('Голос: данные собраны, подключаюсь к голосовому серверу…');
});

// --- Локальные обработчики ---
initLogin();
initChatInput();
initVoiceControls();

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
