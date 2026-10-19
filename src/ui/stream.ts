// Экран чужой демонстрации экрана (focus-режим «как в Discord»): клик по
// стримеру в колонке участников → огромный экран вместо чата и правой колонки.
// Сам видео-поток рисуем на этапе декодера (Фаза 3); сейчас экран показывает
// статус подключения, а звук демки уже идёт через общий mixer.

import { api } from '../api';
import { el } from '../dom';
import type { StreamUpdatePayload } from '../events';
import { state } from '../state';
import { renderMembers } from './members';
import { showError } from './toast';

/** Смотрим ли демонстрацию конкретного участника (строка-переключатель). */
export function isWatchingUser(userId: string): boolean {
  return state.watching?.owner_id === userId;
}

/** Клик по стримеру: сразу открываем экран, параллельно шлём op 20 (Watch Stream).
 *  Подключение занимает секунды — статус на экране двигает stream_update. */
export async function watchStream(
  ownerId: string,
  username: string,
  channelId: string,
): Promise<void> {
  const guildId = state.currentGuildId;
  if (!guildId) return;
  // Повторный клик по тому же стримеру — перестаём смотреть.
  if (isWatchingUser(ownerId)) {
    await stopWatching();
    return;
  }
  // Discord требует находиться в том же голосовом канале, что и стример.
  if (state.currentVoiceChannelId !== channelId) {
    showError('Смотреть демонстрацию можно только из того же голосового канала — сначала зайди в него');
    return;
  }
  if (state.watching) await stopWatching(); // смотрим максимум один стрим
  const stream_key = `guild:${guildId}:${channelId}:${ownerId}`;
  state.watching = { stream_key, owner_id: ownerId, username };
  setStreamOwner(username);
  setStreamStatus('Подключаюсь к демонстрации…');
  el('streamScreen').classList.add('active');
  try {
    await api.watchStream(stream_key);
  } catch (e) {
    closeScreen();
    showError('Не удалось подключиться к демонстрации: ' + e);
  }
}

/** Кнопка «Перестать смотреть» / ESC / повторный клик по стримеру. */
export async function stopWatching(): Promise<void> {
  if (!state.watching) return;
  closeScreen();
  try {
    await api.unwatchStream();
  } catch (e) {
    showError(e);
  }
}

/** STREAM_UPDATE из бэкенда: двигаем статус и закрываем экран, когда демка
 *  кончилась (стример остановился, вышел из канала, обрыв соединения). */
export function handleStreamUpdate(p: StreamUpdatePayload): void {
  if (!state.watching) return; // смотрим не мы — не вмешиваемся
  if (p.stream_key && p.stream_key !== state.watching.stream_key) return;
  if (!p.watching) {
    closeScreen();
    return;
  }
  setStreamStatus('Подключено — видео появится на следующем этапе (декодер)');
}

function closeScreen(): void {
  state.watching = null;
  el('streamScreen').classList.remove('active');
  renderMembers(); // вернуть в колонке строку «Смотреть демонстрацию»
}

function setStreamOwner(name: string): void {
  el('streamOwner').textContent = name;
}

function setStreamStatus(text: string): void {
  el('streamStatus').textContent = text;
}

/** Кнопка закрытия + ESC. Вызывается один раз из app.ts. */
export function initStreamScreen(): void {
  el('streamStop').onclick = () => void stopWatching();
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && state.watching) void stopWatching();
  });
}
