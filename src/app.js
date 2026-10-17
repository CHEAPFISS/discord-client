// Discord Client — фронтенд (Tauri + vanilla JS).
// Общается с бэкендом через window.__TAURI__ (withGlobalTauri: true в tauri.conf.json).

const invoke = window.__TAURI__?.core?.invoke;
const listen = window.__TAURI__?.event?.listen;

const toast = document.getElementById('errorToast');
const statusDot = document.getElementById('statusDot');
const statusText = document.getElementById('statusText');

function showError(msg) {
  if (!toast) return alert(msg);
  toast.textContent = msg;
  toast.style.display = 'block';
  setTimeout(() => { toast.style.display = 'none'; }, 8000);
  toast.onclick = () => { toast.style.display = 'none'; };
}

function setGatewayStatus(status, error) {
  if (!statusDot || !statusText) return;
  statusDot.className = 'status-dot';
  switch (status) {
    case 'connected':
    case 'ready':
      statusDot.classList.add('connected');
      statusText.textContent = 'Подключено к Discord';
      statusText.style.color = '#dcddde';
      break;
    case 'reconnecting':
      statusDot.classList.add('connecting');
      statusText.textContent = error ? 'Переподключение… (' + error + ')' : 'Переподключение…';
      statusText.style.color = '#faa61a';
      break;
    default:
      statusDot.classList.add('disconnected');
      statusText.textContent = 'Нет соединения';
      statusText.style.color = '#8e9297';
  }
}

// --- DOM ---
const loginScreen = document.getElementById('login');
const appEl = document.getElementById('app');
const loginBtn = document.getElementById('loginBtn');
const loginStatus = document.getElementById('loginStatus');
const serversEl = document.getElementById('servers');
const channelsEl = document.getElementById('channels');
const messagesEl = document.getElementById('messages');
const chatHeader = document.getElementById('chatHeader');
const messageInput = document.getElementById('messageInput');
const usernameEl = document.getElementById('username');
const avatarEl = document.getElementById('avatar');
const userAreaNameEl = document.getElementById('userAreaName');
const voiceCall = document.getElementById('voiceCall');
const voiceChannelName = document.getElementById('voiceChannelName');
const hangupBtn = document.getElementById('hangupBtn');
const logoutBtn = document.getElementById('logoutBtn');
const statusEl = document.getElementById('voiceStatus');

let currentGuildId = null;
let currentChannelId = null;
let currentVoiceChannelId = null;
const unread = new Map();   // channelId => количество непрочитанных
const channelEls = {};      // channelId => DOM-элемент канала

// --- Экран входа ---

function showApp(username) {
  loginScreen.style.display = 'none';
  appEl.style.display = 'flex';
  usernameEl.textContent = username;
  userAreaNameEl.textContent = username;
  avatarEl.textContent = username.charAt(0).toUpperCase();
  loadGuilds();
}

async function tryRestoreSession() {
  try {
    const me = await invoke('get_me');
    showApp(me.username);
    // запускаем Gateway WebSocket для живых сообщений
    invoke('connect_gateway').catch(e => console.warn('gateway:', e));
  } catch (e) {
    // не залогинен — остаёмся на экране входа
  }
}

loginBtn.addEventListener('click', async () => {
  loginBtn.disabled = true;
  loginStatus.textContent = 'Открываю Discord в браузере…';
  try {
    const res = await invoke('start_login');
    showApp(res.username);
    invoke('connect_gateway').catch(e => console.warn('gateway:', e));
  } catch (e) {
    loginStatus.textContent = String(e);
    showError(String(e));
  } finally {
    loginBtn.disabled = false;
  }
});

const tokenInput = document.getElementById('tokenInput');
const tokenBtn = document.getElementById('tokenBtn');

tokenBtn.addEventListener('click', async () => {
  const token = tokenInput.value.trim();
  if (!token) return showError('Вставь токен в поле');
  tokenBtn.disabled = true;
  loginStatus.textContent = 'Проверяю токен…';
  try {
    const res = await invoke('login_with_token', { token });
    tokenInput.value = '';
    showApp(res.username);
    invoke('connect_gateway').catch(e => console.warn('gateway:', e));
  } catch (e) {
    loginStatus.textContent = String(e);
    showError(String(e));
  } finally {
    tokenBtn.disabled = false;
  }
});

// --- Серверы / каналы ---

function serverAvatar(guild) {
  if (guild.icon) {
    return `https://cdn.discordapp.com/icons/${guild.id}/${guild.icon}.png?size=64`;
  }
  return null;
}

async function loadGuilds() {
  try {
    const guilds = await invoke('get_guilds');
    serversEl.innerHTML = '';
    if (!guilds || guilds.length === 0) {
      const empty = document.createElement('div');
      empty.className = 'empty-servers';
      empty.textContent = 'Серверы не найдены';
      serversEl.appendChild(empty);
      return;
    }
    guilds.forEach(g => {
      const div = document.createElement('div');
      div.className = 'server-item';
      div.title = g.name;
      const imgUrl = serverAvatar(g);
      if (imgUrl) {
        const img = document.createElement('img');
        img.src = imgUrl;
        img.alt = g.name;
        img.onerror = () => {
          div.textContent = g.name.charAt(0).toUpperCase();
          div.classList.add('server-letter');
        };
        div.appendChild(img);
      } else {
        div.textContent = g.name.charAt(0).toUpperCase();
        div.classList.add('server-letter');
      }
      div.onclick = () => {
        document.querySelectorAll('.server-item').forEach(el => el.classList.remove('active'));
        div.classList.add('active');
        loadChannels(g);
      };
      serversEl.appendChild(div);
    });
  } catch (e) {
    showError('Не удалось загрузить серверы: ' + e);
  }
}

function updateBadge(channelId) {
  const el = channelEls[channelId];
  if (!el) return;
  const badge = el.querySelector('.unread-badge');
  const n = unread.get(channelId) || 0;
  if (n > 0) {
    badge.textContent = n > 9 ? '9+' : String(n);
    badge.classList.add('visible');
  } else {
    badge.classList.remove('visible');
  }
}

async function loadChannels(guild) {
  currentGuildId = guild.id;
  channelsEl.innerHTML = '';
  const cat = document.createElement('div');
  cat.className = 'channel-category';
  cat.textContent = guild.name;
  channelsEl.appendChild(cat);

  try {
    const channels = await invoke('get_channels', { guildId: guild.id });
    channels.forEach(c => {
      const div = document.createElement('div');
      div.className = `channel-item ${(c.kind === 2 || c.kind === 13) ? 'voice' : 'text'}`;
      div.dataset.channelId = c.id;
      const nameSpan = document.createElement('span');
      nameSpan.className = 'channel-name';
      nameSpan.textContent = c.name;
      const badge = document.createElement('span');
      badge.className = 'unread-badge';
      div.appendChild(nameSpan);
      div.appendChild(badge);
      channelEls[c.id] = div;
      updateBadge(c.id);

      div.onclick = () => {
        document.querySelectorAll('.channel-item').forEach(el => el.classList.remove('active'));
        div.classList.add('active');
        // сбрасываем непрочитанное для этого канала
        unread.delete(c.id);
        updateBadge(c.id);
        if (c.kind === 2 || c.kind === 13) joinVoice(c);
        else openTextChannel(c);
      };
      channelsEl.appendChild(div);
    });
  } catch (e) {
    showError('Не удалось загрузить каналы: ' + e);
  }
}

// --- Сообщения ---

function fmtTime(ts) {
  if (!ts) return '';
  const d = new Date(ts);
  return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

function escapeHtml(text) {
  const div = document.createElement('div');
  div.textContent = text;
  return div.innerHTML;
}

function clearMessages() {
  messagesEl.innerHTML = '';
  const empty = document.createElement('div');
  empty.className = 'messages-empty';
  empty.textContent = 'Сообщений пока нет';
  messagesEl.appendChild(empty);
}

function renderContent(rawHtml) {
  // Упоминания <@id> → красивая метка @user (без API-разрешения имён)
  return rawHtml.replace(/&lt;@(!?\d+)&gt;/g, '<span class="mention">@$1</span>');
}

function renderMessage(msg, { live = false } = {}) {
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
  if (!live) return;
  // автопрокрутка только если пользователь внизу чата
  const atBottom = messagesEl.scrollTop + messagesEl.clientHeight >= messagesEl.scrollHeight - 40;
  if (atBottom) messagesEl.scrollTop = messagesEl.scrollHeight;
}

async function openTextChannel(channel) {
  chatHeader.textContent = '# ' + channel.name;
  currentChannelId = channel.id;
  messagesEl.innerHTML = '';
  clearMessages();
  try {
    const history = await invoke('get_messages', { channelId: channel.id });
    if (history.length === 0) clearMessages();
    else history.forEach(msg => renderMessage(msg));
  } catch (e) {
    messagesEl.innerHTML = '';
    const div = document.createElement('div');
    div.className = 'messages-empty';
    div.textContent = 'Не удалось загрузить историю: ' + e;
    messagesEl.appendChild(div);
  }
}

function appendLiveMessage(msg) {
  // Считаем непрочитанное, если канал не открыт или сообщение из другого канала
  if (!currentChannelId || msg.channel_id !== currentChannelId) {
    unread.set(msg.channel_id, (unread.get(msg.channel_id) || 0) + 1);
    updateBadge(msg.channel_id);
    return;
  }
  renderMessage(msg, { live: true });
}

// --- Голосовой канал ---

async function joinVoice(channel) {
  currentVoiceChannelId = channel.id;
  voiceChannelName.textContent = channel.name;
  setStatus('Подключаюсь…');
  voiceCall.classList.add('active');
  try {
    await invoke('join_voice_channel', { channelId: channel.id, guildId: currentGuildId });
  } catch (e) {
    setStatus('Ошибка: ' + e);
  }
}

function setStatus(text) {
  statusEl.textContent = text;
}

let micMuted = false;
let deafened = false;

document.getElementById('micBtn').onclick = async () => {
  micMuted = !micMuted;
  try { await invoke('set_self_mute', { muted: micMuted }); } catch (_) {}
  const btn = document.getElementById('micBtn');
  btn.classList.toggle('active', micMuted);
  btn.title = micMuted ? 'Микрофон заглушён' : 'Микрофон';
  btn.textContent = micMuted ? '🔇' : '🎙';
};

document.getElementById('deafBtn').onclick = () => {
  deafened = !deafened;
  const btn = document.getElementById('deafBtn');
  btn.classList.toggle('active', deafened);
  btn.title = deafened ? 'Звук отключён' : 'Звук';
  btn.textContent = deafened ? '🔇' : '🔊';
};

hangupBtn.onclick = async () => {
  try {
    await invoke('leave_voice_channel');
  } catch (_) {}
  currentVoiceChannelId = null;
  setStatus('');
  voiceCall.classList.remove('active');
};

// --- Отправка сообщений ---

messageInput.addEventListener('input', () => {
  messageInput.style.height = 'auto';
  messageInput.style.height = messageInput.scrollHeight + 'px';
});

messageInput.addEventListener('keypress', async (e) => {
  if (e.key === 'Enter' && !e.shiftKey) {
    e.preventDefault();
    const text = messageInput.value.trim();
    if (!text || !currentChannelId) return;
    messageInput.value = '';
    messageInput.style.height = 'auto';
    try {
      const msg = await invoke('send_message', { channelId: currentChannelId, content: text });
      renderMessage(msg, { live: true });
      messagesEl.scrollTop = messagesEl.scrollHeight;
    } catch (err) {
      showError('Не отправилось: ' + err);
    }
  }
});

// --- Живые события от Gateway ---

if (listen) {
  listen('message_create', (event) => {
    appendLiveMessage(event.payload);
  });
  listen('gateway_status', (event) => {
    const p = event.payload || {};
    setGatewayStatus(p.status, p.error);
  });
  listen('voice_status', (event) => {
    const p = event.payload || {};
    if (p.status === 'left') {
      currentVoiceChannelId = null;
      setStatus('');
      voiceCall.classList.remove('active');
    } else if (p.status === 'joining') {
      setStatus('Подключаюсь…');
    } else if (p.status === 'connected') {
      setStatus('Голос: подключено (DAVE/E2EE) 🎧');
    } else if (p.status === 'reconnecting') {
      setStatus('Голос: переподключение…');
    } else if (p.status === 'error') {
      setStatus('Ошибка голоса: ' + (p.error || 'неизвестно'));
    }
  });
  listen('voice_channel_ready', (event) => {
    setStatus('Голос: данные собраны, подключаюсь к голосовому серверу…');
  });
}

logoutBtn.onclick = async () => {
  try { await invoke('logout'); } catch (_) {}
  location.reload();
};

// Автовосстановление сессии при загрузке
tryRestoreSession();
