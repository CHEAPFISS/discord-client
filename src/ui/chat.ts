// Текстовый чат: история, рендер сообщений, отправка.

import { api, type Channel, type ChatMessage } from '../api';
import { el } from '../dom';
import { state } from '../state';
import { showError } from './toast';
import { updateBadge } from './unread';

function fmtTime(ts: string): string {
  if (!ts) return '';
  const d = new Date(ts);
  return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

function escapeHtml(text: string): string {
  const div = document.createElement('div');
  div.textContent = text;
  return div.innerHTML;
}

function renderContent(rawHtml: string): string {
  // Упоминания <@id> → красивая метка @user (без API-разрешения имён)
  return rawHtml.replace(/&lt;@(!?\d+)&gt;/g, '<span class="mention">@$1</span>');
}

function renderMessage(msg: ChatMessage, opts: { live?: boolean } = {}): void {
  const messagesEl = el('messages');
  // Убираем заглушку "пусто" при первом сообщении
  const existingEmpty = messagesEl.querySelector('.messages-empty');
  if (existingEmpty) existingEmpty.remove();

  const div = document.createElement('div');
  div.className = 'message';
  const authorLetter = (msg.author || '?').charAt(0).toUpperCase();
  div.innerHTML = `
    <div class="avatar">${escapeHtml(authorLetter)}</div>
    <div class="bubble">
      <div class="header">
        <span class="author">${escapeHtml(msg.author)}</span>
        <span class="time">${fmtTime(msg.timestamp)}</span>
      </div>
      <div class="text">${renderContent(escapeHtml(msg.content))}</div>
    </div>
  `;
  messagesEl.appendChild(div);
  if (!opts.live) return;
  // автопрокрутка только если пользователь внизу чата
  const atBottom = messagesEl.scrollTop + messagesEl.clientHeight >= messagesEl.scrollHeight - 40;
  if (atBottom) messagesEl.scrollTop = messagesEl.scrollHeight;
}

function clearMessages(): void {
  const messagesEl = el('messages');
  messagesEl.innerHTML = '';
  const empty = document.createElement('div');
  empty.className = 'messages-empty';
  empty.textContent = 'Сообщений пока нет';
  messagesEl.appendChild(empty);
}

export async function openTextChannel(channel: Channel): Promise<void> {
  const messagesEl = el('messages');
  el('chatHeader').textContent = '# ' + channel.name;
  state.currentChannelId = channel.id;
  messagesEl.innerHTML = '';
  clearMessages();
  try {
    const history = await api.getMessages(channel.id);
    if (history.length === 0) clearMessages();
    else history.forEach((msg) => renderMessage(msg));
  } catch (e) {
    messagesEl.innerHTML = '';
    const div = document.createElement('div');
    div.className = 'messages-empty';
    div.textContent = 'Не удалось загрузить историю: ' + e;
    messagesEl.appendChild(div);
  }
}

/** Живое сообщение из Gateway: открытому каналу — рендер, остальным — счётчик. */
export function appendLiveMessage(msg: ChatMessage): void {
  if (!state.currentChannelId || msg.channel_id !== state.currentChannelId) {
    state.unread.set(msg.channel_id, (state.unread.get(msg.channel_id) || 0) + 1);
    updateBadge(msg.channel_id);
    return;
  }
  renderMessage(msg, { live: true });
}

/** Поле ввода: автовысота и отправка по Enter (Shift+Enter — перенос строки). */
export function initChatInput(): void {
  const messageInput = el<HTMLTextAreaElement>('messageInput');
  const messagesEl = el('messages');

  messageInput.addEventListener('input', () => {
    messageInput.style.height = 'auto';
    messageInput.style.height = messageInput.scrollHeight + 'px';
  });

  messageInput.addEventListener('keypress', async (e) => {
    if (e.key !== 'Enter' || e.shiftKey) return;
    e.preventDefault();
    const text = messageInput.value.trim();
    const channelId = state.currentChannelId;
    if (!text || !channelId) return;
    messageInput.value = '';
    messageInput.style.height = 'auto';
    try {
      const msg = await api.sendMessage(channelId, text);
      renderMessage(msg, { live: true });
      messagesEl.scrollTop = messagesEl.scrollHeight;
    } catch (err) {
      showError('Не отправилось: ' + err);
    }
  });
}
