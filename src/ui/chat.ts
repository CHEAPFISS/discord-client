// Текстовый чат: история, рендер сообщений, отправка.

import { api, type Attachment, type Channel, type ChatMessage, type Embed } from '../api';
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

// Упоминание <@id> → «@ник»: имя берём из списка участников сервера; цвет —
// CSS-переменные --mention-* (настраиваются в одной точке).
function mentionHtml(id: string): string {
  const m = state.members.find((x) => x.user_id === id);
  if (!m) {
    return `<span class="mention">@${escapeHtml(id)}</span>`;
  }
  return `<span class="mention">@${escapeHtml(m.username)}</span>`;
}

function renderContent(rawHtml: string): string {
  return (
    linkify(rawHtml)
      // Упоминания <@id>/<@!id> → @ник с цветом роли
      .replace(/&lt;@(!?)(\d+)&gt;/g, (_, _bang: string, id: string) => mentionHtml(id))
      // Спойлеры ||текст|| — скрыты до клика
      .replace(/\|\|(.+?)\|\|/gs, '<span class="spoiler">$1</span>')
  );
}

/** Голый URL в тексте → кликабельная подсвеченная ссылка (текст уже html-экранирован). */
function linkify(escaped: string): string {
  return escaped.replace(
    /(https?:\/\/[^\s<]+)/g,
    (u) => `<a class="link" href="${u.replace(/"/g, '%22')}">${u}</a>`,
  );
}

const IMAGE_EXT = /\.(png|jpe?g|gif|webp|avif)(\?.*)?$/i;
const VIDEO_EXT = /\.(mp4|webm|mov)(\?.*)?$/i;

function isImage(a: Attachment): boolean {
  return (a.content_type ?? '').startsWith('image/') || IMAGE_EXT.test(a.url);
}

function isVideo(a: Attachment): boolean {
  return (a.content_type ?? '').startsWith('video/') || VIDEO_EXT.test(a.url);
}

/** gifv-эмбед (гифка со ссылки) — как картинка-аттачмент: голая гифка + метка GIF. */
function gifNode(e: Embed): HTMLElement {
  const wrap = document.createElement('div');
  wrap.className = 'gif-item';
  const video = document.createElement('video');
  video.className = 'gif-video';
  video.src = e.video ?? '';
  video.autoplay = true;
  video.muted = true;
  video.playsInline = true;
  video.preload = 'auto';
  // WebKit: loop перезагружает src на петле (видимое мигание) — перезапуск вручную.
  video.onended = () => {
    video.currentTime = 0;
    void video.play().catch(() => {});
  };
  video.onerror = () => wrap.remove();
  wrap.appendChild(video);
  const label = document.createElement('span');
  label.className = 'gif-label';
  label.textContent = 'GIF';
  wrap.appendChild(label);
  return wrap;
}

/** Embed-карточка: цветная полоса, автор/заголовок/описание/поля/картинки. */
function embedNode(e: Embed): HTMLElement {
  const box = document.createElement('div');
  box.className = 'msg-embed';
  if (e.url) box.dataset.url = e.url;
  if (e.color != null) {
    box.style.borderLeftColor = '#' + e.color.toString(16).padStart(6, '0');
  }
  const parts: string[] = [];
  if (e.author) parts.push(`<div class="embed-author">${escapeHtml(e.author)}</div>`);
  if (e.title) {
    const t = escapeHtml(e.title);
    parts.push(
      e.url
        ? `<a class="embed-title" href="${e.url.replace(/"/g, '%22')}">${t}</a>`
        : `<div class="embed-title">${t}</div>`,
    );
  }
  if (e.description) {
    parts.push(`<div class="embed-desc">${linkify(escapeHtml(e.description))}</div>`);
  }
  if (e.fields.length > 0) {
    const fields = e.fields
      .map(
        (f) =>
          `<div class="embed-field${f.inline ? ' inline' : ''}">` +
          `<div class="ef-name">${escapeHtml(f.name)}</div>` +
          `<div class="ef-value">${linkify(escapeHtml(f.value))}</div></div>`,
      )
      .join('');
    parts.push(`<div class="embed-fields">${fields}</div>`);
  }
  if (e.footer) parts.push(`<div class="embed-footer">${escapeHtml(e.footer)}</div>`);
  box.innerHTML = parts.join('');
  if (e.kind === 'video' && e.image) {
    // Видео-превью (YouTube и пр.): картинка + play-иконка, как у официального.
    const media = document.createElement('div');
    media.className = 'embed-media';
    const img = document.createElement('img');
    img.className = 'embed-image';
    img.src = e.image;
    img.loading = 'lazy';
    media.appendChild(img);
    const badge = document.createElement('span');
    badge.className = 'play-badge';
    media.appendChild(badge);
    box.appendChild(media);
  } else if (e.video) {
    // gifv-фолбэк (основной путь — gifNode в renderMessage): mp4 без loop,
    // перезапуск вручную по ended — WebKit на loop перезагружает src (мигание).
    const video = document.createElement('video');
    video.className = 'embed-video';
    video.src = e.video;
    video.autoplay = true;
    video.muted = true;
    video.playsInline = true;
    video.preload = 'auto';
    video.onended = () => {
      video.currentTime = 0;
      void video.play().catch(() => {});
    };
    video.onerror = () => video.remove();
    box.appendChild(video);
  } else if (e.image) {
    const img = document.createElement('img');
    img.className = 'embed-image';
    img.src = e.image;
    img.loading = 'lazy';
    box.appendChild(img);
  }
  // Превью справа — только если нет большого медиа (как в официальном клиенте).
  if (e.thumbnail && !e.image && !e.video) {
    box.classList.add('with-thumb');
    const th = document.createElement('img');
    th.className = 'embed-thumb';
    th.src = e.thumbnail;
    th.loading = 'lazy';
    box.appendChild(th);
  }
  return box;
}

/** Вложения после текста: картинки/GIF и видео — превью, прочее — карточка файла. */
function appendAttachments(bubble: HTMLElement, msg: ChatMessage): void {
  const media = document.createElement('div');
  media.className = 'att-media';
  for (const a of msg.attachments) {
    if (isImage(a)) {
      const img = document.createElement('img');
      img.className = 'att-image';
      img.src = a.url;
      img.alt = a.filename;
      img.loading = 'lazy';
      img.title = 'Открыть в браузере';
      media.appendChild(img);
    } else if (isVideo(a)) {
      const video = document.createElement('video');
      video.className = 'att-video';
      video.src = a.url;
      video.controls = true;
      video.preload = 'metadata';
      media.appendChild(video);
    } else {
      const link = document.createElement('a');
      link.className = 'att-file';
      link.href = a.url;
      link.textContent = `📎 ${a.filename}`;
      bubble.appendChild(link);
    }
  }
  if (media.childElementCount > 0) bubble.appendChild(media);
}

/** Аватар автора: с CDN (gif для анимированных), иначе — буква кружком. */
function avatarNode(msg: ChatMessage): HTMLElement {
  const av = document.createElement('div');
  av.className = 'avatar';
  const letter = (msg.author || '?').charAt(0).toUpperCase();
  if (msg.author_id && msg.author_avatar) {
    const ext = msg.author_avatar.startsWith('a_') ? 'gif' : 'png';
    const img = document.createElement('img');
    img.src = `https://cdn.discordapp.com/avatars/${msg.author_id}/${msg.author_avatar}.${ext}?size=64`;
    img.alt = '';
    img.onerror = () => {
      img.remove();
      av.textContent = letter;
    };
    av.appendChild(img);
  } else {
    av.textContent = letter;
  }
  return av;
}

function renderMessage(msg: ChatMessage, opts: { live?: boolean } = {}): void {
  const messagesEl = el('messages');
  // Убираем заглушку "пусто" при первом сообщении
  const existingEmpty = messagesEl.querySelector('.messages-empty');
  if (existingEmpty) existingEmpty.remove();

  const div = document.createElement('div');
  div.className = 'message';
  div.appendChild(avatarNode(msg));
  const bubble = document.createElement('div');
  bubble.className = 'bubble';
  // Ссылки, породившие gifv-эмбед, из текста убираем — остаётся только гифка
  // (как у официального клиента). Не строгое равенство, а «url внутри строки»
  // (невидимые символы/хвосты не мешают) + голая ссылка на тот же домен.
  let content = msg.content;
  for (const e of msg.embeds) {
    if (e.kind !== 'gifv' || !e.url) continue;
    const gifUrl = e.url;
    const hostname = (u: string): string | null => {
      try {
        return new URL(u).hostname.replace(/^www\./, '');
      } catch {
        return null;
      }
    };
    const gifHost = hostname(gifUrl);
    content = content
      .split('\n')
      .filter((line) => {
        const t = line.trim();
        if (!t) return false;
        if (t.includes(gifUrl)) return false;
        if (/^https?:\/\/\S+$/.test(t) && gifHost && hostname(t) === gifHost) return false;
        return true;
      })
      .join('\n');
  }
  bubble.innerHTML = `
      <div class="header">
        <span class="author">${escapeHtml(msg.author)}</span>
        <span class="time">${fmtTime(msg.timestamp)}</span>
      </div>
      ${content ? `<div class="text">${renderContent(escapeHtml(content))}</div>` : ''}
  `;
  appendAttachments(bubble, msg);
  for (const e of msg.embeds) {
    // Гифки со ссылок — голым медио («GIF»), а не embed-карточкой с пустотой.
    if (e.kind === 'gifv' && e.video) bubble.appendChild(gifNode(e));
    else bubble.appendChild(embedNode(e));
  }
  div.appendChild(bubble);
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

  // Ссылки и картинки — в системный браузер, а не в вебвью.
  messagesEl.addEventListener('click', (e) => {
    const target = e.target as HTMLElement;
    if (target.classList.contains('spoiler')) {
      target.classList.replace('spoiler', 'revealed');
      return;
    }
    if (target instanceof HTMLImageElement && target.classList.contains('att-image')) {
      void api.openUrl(target.src).catch(() => {});
      return;
    }
    const link = target.closest('a[href]') as HTMLAnchorElement | null;
    const emb = target.closest('.msg-embed') as HTMLElement | null;
    // Embed-карточка кликабельна целиком (как у официального): её ссылка;
    // если ссылки нет — сама картинка карточки.
    if (emb && !link) {
      if (emb.dataset.url) {
        e.preventDefault();
        void api.openUrl(emb.dataset.url).catch(() => {});
        return;
      }
      if (target instanceof HTMLImageElement && target.classList.contains('embed-image')) {
        void api.openUrl(target.src).catch(() => {});
        return;
      }
    }
    if (link) {
      e.preventDefault();
      void api.openUrl(link.href).catch(() => {});
    }
  });

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
