// Экран чужой демонстрации экрана (focus-режим «как в Discord»): клик по
// стримеру в колонке участников → огромный экран вместо чата и правой колонки.
// Сам видео-поток рисуем на этапе декодера (Фаза 3); сейчас экран показывает
// статус подключения, а звук демки уже идёт через общий mixer.

import { Channel, invoke } from '@tauri-apps/api/core';
import { api } from '../api';
import { el } from '../dom';
import type { StreamUpdatePayload } from '../events';
import { t } from '../i18n';
import { state } from '../state';
import { renderMembers } from './members';
import { showError } from './toast';

// Рендер видео: пока canvas не получил ни одного кадра — он скрыт и виден
// плейсхолдер со статусом. Первый кадр показывает canvas и прячет плейсхолдер.
let videoCtx: CanvasRenderingContext2D | null = null;
/** Защита от «наезда» кадров: createImageBitmap асинхронен, если предыдущий
 *  кадр ещё декодируется — текущий отбрасывается (на экране всегда свежий). */
let videoFrameInFlight = false;
/** Размер последнего кадра — для вписывания canvas в сцену при resize окна. */
let videoFrameSize: { w: number; h: number } | null = null;
/** Remembered status key so a language switch can re-translate the placeholder. */
let lastStreamStatus: { key: string; vars?: Record<string, string | number> } | null = null;

/** Вписывает canvas в сцену с сохранением пропорций КАДРА: элемент заполняет
 *  максимум доступного места. Без этого canvas жил в «натуральную» величину
 *  (1280×720 CSS-пикселей): на большом окне выглядел маленьким с огромными
 *  чёрными полями, а при низком стартовом разрешении стрима «рос» с каждым
 *  повышением разрешения — вместо того чтобы сразу занять сцену. */
function fitCanvasToStage(): void {
  const canvas = el<HTMLCanvasElement>('streamCanvas');
  const stage = el('streamScreen').querySelector<HTMLElement>('.stream-stage');
  if (!stage || !videoFrameSize) return;
  const sw = stage.clientWidth;
  const sh = stage.clientHeight;
  if (sw <= 0 || sh <= 0) return;
  const scale = Math.min(sw / videoFrameSize.w, sh / videoFrameSize.h);
  canvas.style.width = `${Math.floor(videoFrameSize.w * scale)}px`;
  canvas.style.height = `${Math.floor(videoFrameSize.h * scale)}px`;
}
window.addEventListener('resize', fitCanvasToStage);

/** Создаём бинарный канал кадров и отдаём бэкенду (stream_video_attach).
 *  Бэкенд шлёт ArrayBuffer: [u32 width LE][u32 height LE][jpeg…] — JPEG
 *  (~250 КБ на кадр 1080p вместо 8.3 МБ сырых RGBA), декодер асинхронный. */
function attachVideoChannel(): void {
  const channel = new Channel<ArrayBuffer>();
  channel.onmessage = (buf) => {
    const view = new DataView(buf);
    const width = view.getUint32(0, true);
    const height = view.getUint32(4, true);
    drawJpegFrame(width, height, buf.slice(8));
  };
  invoke('stream_video_attach', { channel }).catch((e) => {
    console.warn('stream_video_attach:', e);
  });
}

/** Кадр-JPEG из бинарного канала: createImageBitmap → canvas. Декодирование
 *  JPEG вынесено воркером WebKit (не блокирует UI-поток). */
async function drawJpegFrame(width: number, height: number, jpeg: ArrayBuffer): Promise<void> {
  if (!state.watching) return; // не смотрим — кадры бэкенд-остатки, игнор
  if (videoFrameInFlight) return; // предыдущий кадр ещё декодируется — берём свежий позже
  videoFrameInFlight = true;
  try {
    const bitmap = await createImageBitmap(new Blob([jpeg], { type: 'image/jpeg' }));
    const canvas = el<HTMLCanvasElement>('streamCanvas');
    const stage = el('streamScreen').querySelector('.stream-stage');
    if (canvas.width !== width || canvas.height !== height) {
      canvas.width = width;
      canvas.height = height;
      videoCtx = null; // размер сменился — контекст пересоздаём
      videoFrameSize = { w: width, h: height };
      fitCanvasToStage();
    }
    if (!videoCtx) videoCtx = canvas.getContext('2d');
    if (!videoCtx) return;
    videoCtx.drawImage(bitmap, 0, 0, width, height);
    bitmap.close();
    // Показываем видео, убирая плейсхолдер (только при первом кадре).
    if (canvas.hidden) {
      canvas.hidden = false;
      stage?.classList.add('has-video');
      fitCanvasToStage(); // сцена уже видима — вписываем кадр в неё
    }
  } catch {
    // Битый/устаревший кадр — молча пропускаем.
  } finally {
    videoFrameInFlight = false;
  }
}

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
    showError(t('stream.onlySameChannel'));
    return;
  }
  if (state.watching) await stopWatching(); // смотрим максимум один стрим
  const stream_key = `guild:${guildId}:${channelId}:${ownerId}`;
  state.watching = { stream_key, owner_id: ownerId, username };
  setStreamOwner(username);
  setStreamStatusKey('stream.connecting');
  el('streamScreen').classList.add('active');
  attachVideoChannel(); // бинарный канал кадров — бэкенд начнёт слать пиксели
  try {
    await api.watchStream(stream_key);
  } catch (e) {
    closeScreen();
    showError(t('stream.connectError', { error: String(e) }));
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
  setStreamStatusKey('stream.connected');
}

function closeScreen(): void {
  state.watching = null;
  el('streamScreen').classList.remove('active');
  // Сбрасываем видео: прячем canvas и возвращаем плейсхолдер для следующего стрима.
  const canvas = el<HTMLCanvasElement>('streamCanvas');
  canvas.hidden = true;
  videoCtx = null;
  el('streamScreen').querySelector('.stream-stage')?.classList.remove('has-video');
  renderMembers(); // вернуть в колонке строку «Смотреть демонстрацию»
}

function setStreamOwner(name: string): void {
  el('streamOwner').textContent = name;
}

function setStreamStatusKey(key: string, vars?: Record<string, string | number>): void {
  lastStreamStatus = { key, vars };
  el('streamStatus').textContent = t(key, vars);
}

/** Re-applies the current language to the stream placeholder. */
export function refreshStreamUi(): void {
  if (lastStreamStatus) {
    el('streamStatus').textContent = t(lastStreamStatus.key, lastStreamStatus.vars);
  }
}

/** Кнопка закрытия + ESC. Вызывается один раз из app.ts. */
export function initStreamScreen(): void {
  el('streamStop').onclick = () => void stopWatching();
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && state.watching) void stopWatching();
  });
}
