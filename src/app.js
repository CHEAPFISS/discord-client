// node_modules/@tauri-apps/api/external/tslib/tslib.es6.js
function __classPrivateFieldGet(receiver, state2, kind, f) {
  if (kind === "a" && !f) throw new TypeError("Private accessor was defined without a getter");
  if (typeof state2 === "function" ? receiver !== state2 || !f : !state2.has(receiver)) throw new TypeError("Cannot read private member from an object whose class did not declare it");
  return kind === "m" ? f : kind === "a" ? f.call(receiver) : f ? f.value : state2.get(receiver);
}
function __classPrivateFieldSet(receiver, state2, value, kind, f) {
  if (kind === "m") throw new TypeError("Private method is not writable");
  if (kind === "a" && !f) throw new TypeError("Private accessor was defined without a setter");
  if (typeof state2 === "function" ? receiver !== state2 || !f : !state2.has(receiver)) throw new TypeError("Cannot write private member to an object whose class did not declare it");
  return kind === "a" ? f.call(receiver, value) : f ? f.value = value : state2.set(receiver, value), value;
}

// node_modules/@tauri-apps/api/core.js
var _Channel_instances;
var _Channel_onmessage;
var _Channel_nextMessageIndex;
var _Channel_pendingMessages;
var _Channel_messageEndIndex;
var _Channel_deliver;
var _Resource_rid;
var SERIALIZE_TO_IPC_FN = "__TAURI_TO_IPC_KEY__";
function transformCallback(callback, once = false) {
  return window.__TAURI_INTERNALS__.transformCallback(callback, once);
}
var Channel = class {
  constructor(onmessage) {
    _Channel_instances.add(this);
    _Channel_onmessage.set(this, void 0);
    _Channel_nextMessageIndex.set(this, 0);
    _Channel_pendingMessages.set(this, []);
    _Channel_messageEndIndex.set(this, void 0);
    __classPrivateFieldSet(this, _Channel_onmessage, onmessage || (() => {
    }), "f");
    this.id = transformCallback((rawMessage) => {
      const index = rawMessage.index;
      if ("end" in rawMessage) {
        if (index == __classPrivateFieldGet(this, _Channel_nextMessageIndex, "f")) {
          this.cleanupCallback();
        } else {
          __classPrivateFieldSet(this, _Channel_messageEndIndex, index, "f");
        }
        return;
      }
      const message = rawMessage.message;
      if (index == __classPrivateFieldGet(this, _Channel_nextMessageIndex, "f")) {
        __classPrivateFieldGet(this, _Channel_instances, "m", _Channel_deliver).call(this, message);
        __classPrivateFieldSet(this, _Channel_nextMessageIndex, __classPrivateFieldGet(this, _Channel_nextMessageIndex, "f") + 1, "f");
        while (__classPrivateFieldGet(this, _Channel_nextMessageIndex, "f") in __classPrivateFieldGet(this, _Channel_pendingMessages, "f")) {
          const message2 = __classPrivateFieldGet(this, _Channel_pendingMessages, "f")[__classPrivateFieldGet(this, _Channel_nextMessageIndex, "f")];
          __classPrivateFieldGet(this, _Channel_instances, "m", _Channel_deliver).call(this, message2);
          delete __classPrivateFieldGet(this, _Channel_pendingMessages, "f")[__classPrivateFieldGet(this, _Channel_nextMessageIndex, "f")];
          __classPrivateFieldSet(this, _Channel_nextMessageIndex, __classPrivateFieldGet(this, _Channel_nextMessageIndex, "f") + 1, "f");
        }
        if (__classPrivateFieldGet(this, _Channel_nextMessageIndex, "f") === __classPrivateFieldGet(this, _Channel_messageEndIndex, "f")) {
          this.cleanupCallback();
        }
      } else {
        __classPrivateFieldGet(this, _Channel_pendingMessages, "f")[index] = message;
      }
    });
  }
  cleanupCallback() {
    window.__TAURI_INTERNALS__.unregisterCallback(this.id);
  }
  /**
   * The handler called for every message sent by the Rust side of this channel.
   *
   * Assigning a new handler replaces the previous one; messages that arrived
   * before a handler was set are not replayed, so set it (or pass it to the
   * constructor) before sending the channel to the backend.
   */
  set onmessage(handler) {
    __classPrivateFieldSet(this, _Channel_onmessage, handler, "f");
  }
  get onmessage() {
    return __classPrivateFieldGet(this, _Channel_onmessage, "f");
  }
  [(_Channel_onmessage = /* @__PURE__ */ new WeakMap(), _Channel_nextMessageIndex = /* @__PURE__ */ new WeakMap(), _Channel_pendingMessages = /* @__PURE__ */ new WeakMap(), _Channel_messageEndIndex = /* @__PURE__ */ new WeakMap(), _Channel_instances = /* @__PURE__ */ new WeakSet(), _Channel_deliver = function _Channel_deliver2(message) {
    try {
      __classPrivateFieldGet(this, _Channel_onmessage, "f").call(this, message);
    } catch (error) {
      queueMicrotask(() => {
        throw error;
      });
    }
  }, SERIALIZE_TO_IPC_FN)]() {
    return `__CHANNEL__:${this.id}`;
  }
  toJSON() {
    return this[SERIALIZE_TO_IPC_FN]();
  }
};
async function invoke(cmd, args = {}, options) {
  return window.__TAURI_INTERNALS__.invoke(cmd, args, options);
}
var Resource = class {
  get rid() {
    return __classPrivateFieldGet(this, _Resource_rid, "f");
  }
  constructor(rid) {
    _Resource_rid.set(this, void 0);
    __classPrivateFieldSet(this, _Resource_rid, rid, "f");
  }
  /**
   * Destroys and cleans up this resource from memory.
   * **You should not call any method on this object anymore and should drop any reference to it.**
   *
   * @remarks Uses the `core:resources:allow-close` permission, which is part of
   * the `core:resources:default` permission set enabled by default.
   */
  async close() {
    return invoke("plugin:resources|close", {
      rid: this.rid
    });
  }
  async [(_Resource_rid = /* @__PURE__ */ new WeakMap(), Symbol.asyncDispose)]() {
    await this.close();
  }
};

// src/api.ts
var api = {
  // --- Вход ---
  getMe: () => invoke("get_me"),
  startLogin: () => invoke("start_login"),
  loginWithToken: (token) => invoke("login_with_token", { token }),
  logout: () => invoke("logout"),
  // --- Gateway ---
  connectGateway: () => invoke("connect_gateway"),
  // --- REST Discord ---
  getGuilds: () => invoke("get_guilds"),
  getChannels: (guildId) => invoke("get_channels", { guildId }),
  getGuildMembers: (guildId, channelId) => invoke("get_guild_members", { guildId, channelId }),
  getGuildRoles: (guildId) => invoke("get_guild_roles", { guildId }),
  getGuildActivity: (guildId) => invoke("get_guild_activity", { guildId }),
  getMessages: (channelId) => invoke("get_messages", { channelId }),
  sendMessage: (channelId, content) => invoke("send_message", { channelId, content }),
  // --- Голос ---
  joinVoice: (channelId, guildId) => invoke("join_voice_channel", { channelId, guildId }),
  leaveVoice: () => invoke("leave_voice_channel"),
  setSelfMute: (muted) => invoke("set_self_mute", { muted }),
  setSelfDeaf: (deaf) => invoke("set_self_deaf", { deaf })
};

// src/dom.ts
function el(id) {
  const node = document.getElementById(id);
  if (!node) throw new Error(`\u042D\u043B\u0435\u043C\u0435\u043D\u0442 #${id} \u043D\u0435 \u043D\u0430\u0439\u0434\u0435\u043D \u0432 index.html`);
  return node;
}

// node_modules/@tauri-apps/api/event.js
var TauriEvent;
(function(TauriEvent2) {
  TauriEvent2["WINDOW_RESIZED"] = "tauri://resize";
  TauriEvent2["WINDOW_MOVED"] = "tauri://move";
  TauriEvent2["WINDOW_CLOSE_REQUESTED"] = "tauri://close-requested";
  TauriEvent2["WINDOW_DESTROYED"] = "tauri://destroyed";
  TauriEvent2["WINDOW_FOCUS"] = "tauri://focus";
  TauriEvent2["WINDOW_BLUR"] = "tauri://blur";
  TauriEvent2["WINDOW_SCALE_FACTOR_CHANGED"] = "tauri://scale-change";
  TauriEvent2["WINDOW_THEME_CHANGED"] = "tauri://theme-changed";
  TauriEvent2["WINDOW_CREATED"] = "tauri://window-created";
  TauriEvent2["WINDOW_SUSPENDED"] = "tauri://suspended";
  TauriEvent2["WINDOW_RESUMED"] = "tauri://resumed";
  TauriEvent2["WEBVIEW_CREATED"] = "tauri://webview-created";
  TauriEvent2["DRAG_ENTER"] = "tauri://drag-enter";
  TauriEvent2["DRAG_OVER"] = "tauri://drag-over";
  TauriEvent2["DRAG_DROP"] = "tauri://drag-drop";
  TauriEvent2["DRAG_LEAVE"] = "tauri://drag-leave";
})(TauriEvent || (TauriEvent = {}));
async function _unlisten(event, eventId) {
  window.__TAURI_EVENT_PLUGIN_INTERNALS__.unregisterListener(event, eventId);
  await invoke("plugin:event|unlisten", {
    event,
    eventId
  });
}
async function listen(event, handler, options) {
  var _a;
  const target = typeof (options === null || options === void 0 ? void 0 : options.target) === "string" ? { kind: "AnyLabel", label: options.target } : (_a = options === null || options === void 0 ? void 0 : options.target) !== null && _a !== void 0 ? _a : { kind: "Any" };
  return invoke("plugin:event|listen", {
    event,
    target,
    handler: transformCallback(handler)
  }).then((eventId) => {
    return async () => _unlisten(event, eventId);
  });
}

// src/events.ts
function subscribe(event, cb) {
  listen(event, (e) => cb(e.payload)).catch((err) => {
    console.warn(`listen(${event}):`, err);
  });
}
var onMessageCreate = (cb) => subscribe("message_create", cb);
var onGatewayStatus = (cb) => subscribe("gateway_status", cb);
var onVoiceStatus = (cb) => subscribe("voice_status", cb);
var onVoiceChannelReady = (cb) => subscribe("voice_channel_ready", cb);
var onPresenceUpdate = (cb) => subscribe("presence_update", cb);
var onVoiceMemberUpdate = (cb) => subscribe("voice_member_update", cb);

// src/state.ts
var state = {
  /** Открытый сервер (нужен для join_voice_channel). */
  currentGuildId: null,
  /** Открытый текстовый канал. */
  currentChannelId: null,
  /** Активный голосовой канал (запоминаем для UI). */
  currentVoiceChannelId: null,
  /** Локальные флаги кнопок голосовой панели. */
  micMuted: false,
  deafened: false,
  /** channelId → количество непрочитанных. */
  unread: /* @__PURE__ */ new Map(),
  /** channelId → DOM-элемент канала (для бейджей). */
  channelEls: {},
  // --- Участники сервера (правая колонка) ---
  /** Каналы текущего сервера (для имён голосовых каналов в списке). */
  channels: [],
  /** Участники текущего сервера. */
  members: [],
  /** Группы участников в порядке отображения Discord (hoist-роли + online/offline). */
  memberGroups: [],
  /** Роли текущего сервера (для группировки). */
  roles: [],
  /** user_id → presence-статус (online/idle/dnd/offline). */
  presences: /* @__PURE__ */ new Map(),
  /** user_id → channel_id (кто в каком голосовом канале). */
  memberVoice: /* @__PURE__ */ new Map(),
  /** channelId → контейнер под голосовым каналом (мини-список участников). */
  voiceMemberEls: {},
  /** Сервер, для которого сейчас загружены участники. */
  membersGuildId: null,
  /** Идёт загрузка участников (не перерисовываем промежуточно). */
  membersLoading: false
};

// src/ui/toast.ts
function showError(msg) {
  const text = String(msg);
  const toast = document.getElementById("errorToast");
  if (!toast) {
    alert(text);
    return;
  }
  toast.textContent = text;
  toast.style.display = "block";
  setTimeout(() => {
    toast.style.display = "none";
  }, 8e3);
  toast.onclick = () => {
    toast.style.display = "none";
  };
}

// src/ui/unread.ts
function updateBadge(channelId) {
  const channelEl = state.channelEls[channelId];
  if (!channelEl) return;
  const badge = channelEl.querySelector(".unread-badge");
  if (!badge) return;
  const n = state.unread.get(channelId) || 0;
  if (n > 0) {
    badge.textContent = n > 9 ? "9+" : String(n);
    badge.classList.add("visible");
  } else {
    badge.classList.remove("visible");
  }
}

// src/ui/chat.ts
function fmtTime(ts) {
  if (!ts) return "";
  const d = new Date(ts);
  return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}
function escapeHtml(text) {
  const div = document.createElement("div");
  div.textContent = text;
  return div.innerHTML;
}
function renderContent(rawHtml) {
  return rawHtml.replace(/&lt;@(!?\d+)&gt;/g, '<span class="mention">@$1</span>');
}
function renderMessage(msg, opts = {}) {
  const messagesEl = el("messages");
  const existingEmpty = messagesEl.querySelector(".messages-empty");
  if (existingEmpty) existingEmpty.remove();
  const div = document.createElement("div");
  div.className = "message";
  const authorLetter = (msg.author || "?").charAt(0).toUpperCase();
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
  const atBottom = messagesEl.scrollTop + messagesEl.clientHeight >= messagesEl.scrollHeight - 40;
  if (atBottom) messagesEl.scrollTop = messagesEl.scrollHeight;
}
function clearMessages() {
  const messagesEl = el("messages");
  messagesEl.innerHTML = "";
  const empty = document.createElement("div");
  empty.className = "messages-empty";
  empty.textContent = "\u0421\u043E\u043E\u0431\u0449\u0435\u043D\u0438\u0439 \u043F\u043E\u043A\u0430 \u043D\u0435\u0442";
  messagesEl.appendChild(empty);
}
async function openTextChannel(channel) {
  const messagesEl = el("messages");
  el("chatHeader").textContent = "# " + channel.name;
  state.currentChannelId = channel.id;
  messagesEl.innerHTML = "";
  clearMessages();
  try {
    const history = await api.getMessages(channel.id);
    if (history.length === 0) clearMessages();
    else history.forEach((msg) => renderMessage(msg));
  } catch (e) {
    messagesEl.innerHTML = "";
    const div = document.createElement("div");
    div.className = "messages-empty";
    div.textContent = "\u041D\u0435 \u0443\u0434\u0430\u043B\u043E\u0441\u044C \u0437\u0430\u0433\u0440\u0443\u0437\u0438\u0442\u044C \u0438\u0441\u0442\u043E\u0440\u0438\u044E: " + e;
    messagesEl.appendChild(div);
  }
}
function appendLiveMessage(msg) {
  if (!state.currentChannelId || msg.channel_id !== state.currentChannelId) {
    state.unread.set(msg.channel_id, (state.unread.get(msg.channel_id) || 0) + 1);
    updateBadge(msg.channel_id);
    return;
  }
  renderMessage(msg, { live: true });
}
function initChatInput() {
  const messageInput = el("messageInput");
  const messagesEl = el("messages");
  messageInput.addEventListener("input", () => {
    messageInput.style.height = "auto";
    messageInput.style.height = messageInput.scrollHeight + "px";
  });
  messageInput.addEventListener("keypress", async (e) => {
    if (e.key !== "Enter" || e.shiftKey) return;
    e.preventDefault();
    const text = messageInput.value.trim();
    const channelId = state.currentChannelId;
    if (!text || !channelId) return;
    messageInput.value = "";
    messageInput.style.height = "auto";
    try {
      const msg = await api.sendMessage(channelId, text);
      renderMessage(msg, { live: true });
      messagesEl.scrollTop = messagesEl.scrollHeight;
    } catch (err) {
      showError("\u041D\u0435 \u043E\u0442\u043F\u0440\u0430\u0432\u0438\u043B\u043E\u0441\u044C: " + err);
    }
  });
}

// src/ui/members.ts
function avatarUrl(m) {
  if (!m.avatar) return null;
  const ext = m.avatar.startsWith("a_") ? "gif" : "png";
  return `https://cdn.discordapp.com/avatars/${m.user_id}/${m.avatar}.${ext}?size=64`;
}
function statusOf(userId) {
  return state.presences.get(userId)?.status ?? "offline";
}
function activityLines(userId) {
  const p = state.presences.get(userId);
  if (!p || p.status === "offline") return [];
  const custom = p.activities.find((a) => a.kind === 4);
  if (custom) {
    const text = [custom.emoji, custom.state].filter(Boolean).join(" ").trim();
    if (text) return [{ text }];
  }
  const act = p.activities.find((a) => a.kind !== 4 && a.name);
  if (!act) return [];
  const prefixes = {
    0: "\u0418\u0433\u0440\u0430\u0435\u0442 \u0432",
    1: "\u0421\u0442\u0440\u0438\u043C\u0438\u0442",
    2: "\u0421\u043B\u0443\u0448\u0430\u0435\u0442",
    3: "\u0421\u043C\u043E\u0442\u0440\u0438\u0442",
    5: "\u0421\u043E\u0440\u0435\u0432\u043D\u0443\u0435\u0442\u0441\u044F \u0432"
  };
  const extra = [act.details, act.state].filter(Boolean).join(" \u2014 ");
  return [
    {
      text: `${prefixes[act.kind] ?? "\u0418\u0433\u0440\u0430\u0435\u0442 \u0432"} ${act.name}`,
      title: extra ? `${act.name} \u2014 ${extra}` : void 0
    }
  ];
}
function roleColor(color) {
  return color ? "#" + color.toString(16).padStart(6, "0") : null;
}
async function loadMembers(guildId, channelId) {
  state.membersGuildId = guildId;
  state.membersLoading = true;
  state.members = [];
  state.memberGroups = [];
  state.roles = [];
  const membersEl = el("members");
  membersEl.classList.add("active");
  membersEl.innerHTML = "";
  const loading = document.createElement("div");
  loading.className = "members-empty";
  loading.textContent = "\u0417\u0430\u0433\u0440\u0443\u0436\u0430\u044E \u0443\u0447\u0430\u0441\u0442\u043D\u0438\u043A\u043E\u0432\u2026";
  membersEl.appendChild(loading);
  try {
    const groups = await api.getGuildMembers(guildId, channelId);
    if (state.membersGuildId !== guildId) return;
    const [roles, activity] = await Promise.all([
      api.getGuildRoles(guildId),
      api.getGuildActivity(guildId)
    ]);
    if (state.membersGuildId !== guildId) return;
    state.memberGroups = groups;
    state.members = groups.flatMap((g) => g.members);
    state.roles = roles;
    state.presences = new Map(Object.entries(activity.presences));
    state.memberVoice = new Map(Object.entries(activity.voice));
    state.membersLoading = false;
    renderMembers();
    renderVoiceMembers();
  } catch (e) {
    if (state.membersGuildId !== guildId) return;
    state.membersLoading = false;
    membersEl.innerHTML = "";
    const err = document.createElement("div");
    err.className = "members-empty";
    err.textContent = "\u041D\u0435 \u0443\u0434\u0430\u043B\u043E\u0441\u044C \u0437\u0430\u0433\u0440\u0443\u0437\u0438\u0442\u044C \u0443\u0447\u0430\u0441\u0442\u043D\u0438\u043A\u043E\u0432: " + e;
    membersEl.appendChild(err);
  }
}
function handlePresenceUpdate(p) {
  if (p.guild_id !== state.membersGuildId) return;
  state.presences.set(p.user_id, { status: p.status, activities: p.activities });
  if (!state.membersLoading) renderMembers();
}
function handleVoiceMemberUpdate(p) {
  if (p.guild_id !== state.membersGuildId) return;
  if (p.channel_id) state.memberVoice.set(p.user_id, p.channel_id);
  else state.memberVoice.delete(p.user_id);
  if (!state.membersLoading) {
    renderMembers();
    renderVoiceMembers();
  }
}
function renderMembers() {
  const membersEl = el("members");
  membersEl.innerHTML = "";
  if (state.members.length === 0) {
    const empty = document.createElement("div");
    empty.className = "members-empty";
    empty.textContent = "\u041D\u0435\u0442 \u0443\u0447\u0430\u0441\u0442\u043D\u0438\u043A\u043E\u0432";
    membersEl.appendChild(empty);
    return;
  }
  const groups = [];
  const roleById = new Map(state.roles.map((r) => [r.id, r]));
  const rest = [];
  for (const g of state.memberGroups) {
    if (g.id === "online" || g.id === "offline") {
      rest.push(...g.members);
      continue;
    }
    if (g.members.length === 0) continue;
    const role = roleById.get(g.id);
    groups.push({
      name: role?.name ?? "\u0423\u0447\u0430\u0441\u0442\u043D\u0438\u043A\u0438",
      color: role ? roleColor(role.color) : null,
      members: g.members
    });
  }
  const online = rest.filter((m) => statusOf(m.user_id) !== "offline");
  const offline = rest.filter((m) => statusOf(m.user_id) === "offline");
  if (online.length > 0) groups.push({ name: "\u0412 \u0441\u0435\u0442\u0438", color: null, members: online });
  if (offline.length > 0) groups.push({ name: "\u041D\u0435 \u0432 \u0441\u0435\u0442\u0438", color: null, members: offline });
  for (const g of groups) {
    const groupEl = document.createElement("div");
    groupEl.className = "member-group";
    const title = document.createElement("div");
    title.className = "member-group-title";
    title.textContent = `${g.name} \u2014 ${g.members.length}`;
    if (g.color) title.style.color = g.color;
    groupEl.appendChild(title);
    for (const m of g.members) groupEl.appendChild(memberRow(m));
    membersEl.appendChild(groupEl);
  }
}
function memberRow(m) {
  const row = document.createElement("div");
  row.className = "member";
  const av = document.createElement("div");
  av.className = "member-avatar";
  const url = avatarUrl(m);
  if (url) {
    const img = document.createElement("img");
    img.src = url;
    img.alt = "";
    img.onerror = () => {
      img.remove();
      av.prepend(letterSpan(m.username));
    };
    av.appendChild(img);
  } else {
    av.appendChild(letterSpan(m.username));
  }
  const dot = document.createElement("span");
  dot.className = `presence ${statusOf(m.user_id)}`;
  av.appendChild(dot);
  const body = document.createElement("div");
  body.className = "member-body";
  const name = document.createElement("div");
  name.className = "member-name";
  const nameText = document.createElement("span");
  nameText.textContent = m.username;
  name.appendChild(nameText);
  if (m.bot) {
    const bot = document.createElement("span");
    bot.className = "bot-badge";
    bot.textContent = "\u0411\u041E\u0422";
    name.appendChild(bot);
  }
  body.appendChild(name);
  const lines = activityLines(m.user_id);
  if (lines.length > 0) {
    for (const line of lines) {
      const sub = document.createElement("div");
      sub.className = "member-sub";
      sub.textContent = line.text;
      if (line.title) sub.title = line.title;
      body.appendChild(sub);
    }
  } else if (state.memberVoice.has(m.user_id)) {
    const sub = document.createElement("div");
    sub.className = "member-sub";
    sub.textContent = "\u{1F50A} \u0412 \u0433\u043E\u043B\u043E\u0441\u043E\u0432\u043E\u043C \u0447\u0430\u0442\u0435";
    body.appendChild(sub);
  }
  row.appendChild(av);
  row.appendChild(body);
  return row;
}
function letterSpan(name, cls = "member-letter") {
  const span = document.createElement("span");
  span.className = cls;
  span.textContent = name.charAt(0).toUpperCase();
  return span;
}
function renderVoiceMembers() {
  for (const [channelId, container] of Object.entries(state.voiceMemberEls)) {
    container.innerHTML = "";
    const rows = [...state.memberVoice.entries()].filter(([, cid]) => cid === channelId).map(([uid]) => state.members.find((m) => m.user_id === uid)).filter((m) => m !== void 0).sort((a, b) => a.username.localeCompare(b.username, "ru"));
    for (const m of rows) container.appendChild(voiceMemberRow(m));
  }
}
function voiceMemberRow(m) {
  const row = document.createElement("div");
  row.className = "voice-member";
  const av = document.createElement("div");
  av.className = "voice-member-avatar";
  const url = avatarUrl(m);
  if (url) {
    const img = document.createElement("img");
    img.src = url;
    img.alt = "";
    img.onerror = () => {
      img.remove();
      av.prepend(letterSpan(m.username, "voice-member-letter"));
    };
    av.appendChild(img);
  } else {
    av.appendChild(letterSpan(m.username, "voice-member-letter"));
  }
  const dot = document.createElement("span");
  dot.className = `presence ${statusOf(m.user_id)}`;
  av.appendChild(dot);
  const name = document.createElement("span");
  name.className = "voice-member-name";
  name.textContent = m.username;
  row.appendChild(av);
  row.appendChild(name);
  return row;
}

// src/ui/voice.ts
function setVoiceStatus(text) {
  el("voiceStatus").textContent = text;
}
async function joinVoice(channel) {
  const guildId = state.currentGuildId;
  if (!guildId) return;
  state.currentVoiceChannelId = channel.id;
  el("voiceChannelName").textContent = channel.name;
  setVoiceStatus("\u041F\u043E\u0434\u043A\u043B\u044E\u0447\u0430\u044E\u0441\u044C\u2026");
  el("voiceCall").classList.add("active");
  try {
    await api.joinVoice(channel.id, guildId);
  } catch (e) {
    setVoiceStatus("\u041E\u0448\u0438\u0431\u043A\u0430: " + e);
  }
}
function paintVoiceButtons() {
  const mic = el("micBtn");
  mic.classList.toggle("active", state.micMuted);
  mic.title = state.micMuted ? "\u041C\u0438\u043A\u0440\u043E\u0444\u043E\u043D \u0437\u0430\u0433\u043B\u0443\u0448\u0451\u043D" : "\u041C\u0438\u043A\u0440\u043E\u0444\u043E\u043D";
  mic.textContent = state.micMuted ? "\u{1F507}" : "\u{1F399}";
  const deaf = el("deafBtn");
  deaf.classList.toggle("active", state.deafened);
  deaf.title = state.deafened ? "\u0417\u0432\u0443\u043A \u043E\u0442\u043A\u043B\u044E\u0447\u0451\u043D" : "\u0417\u0432\u0443\u043A";
  deaf.textContent = state.deafened ? "\u{1F507}" : "\u{1F50A}";
}
function initVoiceControls() {
  el("micBtn").onclick = async () => {
    if (state.deafened) {
      state.deafened = false;
      state.micMuted = false;
      try {
        await api.setSelfDeaf(false);
      } catch {
      }
      paintVoiceButtons();
      return;
    }
    state.micMuted = !state.micMuted;
    try {
      await api.setSelfMute(state.micMuted);
    } catch {
    }
    paintVoiceButtons();
  };
  el("deafBtn").onclick = async () => {
    state.deafened = !state.deafened;
    try {
      await api.setSelfDeaf(state.deafened);
    } catch {
    }
    state.micMuted = state.deafened;
    paintVoiceButtons();
  };
  el("hangupBtn").onclick = async () => {
    try {
      await api.leaveVoice();
    } catch {
    }
    state.currentVoiceChannelId = null;
    setVoiceStatus("");
    el("voiceCall").classList.remove("active");
  };
}
function handleVoiceStatus(p) {
  if (p.status === "left") {
    state.currentVoiceChannelId = null;
    setVoiceStatus("");
    el("voiceCall").classList.remove("active");
    state.micMuted = false;
    state.deafened = false;
    paintVoiceButtons();
  } else if (p.status === "joining") {
    setVoiceStatus("\u041F\u043E\u0434\u043A\u043B\u044E\u0447\u0430\u044E\u0441\u044C\u2026");
  } else if (p.status === "connected") {
    setVoiceStatus("\u0413\u043E\u043B\u043E\u0441: \u043F\u043E\u0434\u043A\u043B\u044E\u0447\u0435\u043D\u043E (DAVE/E2EE) \u{1F3A7}");
  } else if (p.status === "reconnecting") {
    setVoiceStatus("\u0413\u043E\u043B\u043E\u0441: \u043F\u0435\u0440\u0435\u043F\u043E\u0434\u043A\u043B\u044E\u0447\u0435\u043D\u0438\u0435\u2026");
  } else if (p.status === "error") {
    setVoiceStatus("\u041E\u0448\u0438\u0431\u043A\u0430 \u0433\u043E\u043B\u043E\u0441\u0430: " + (p.error || "\u043D\u0435\u0438\u0437\u0432\u0435\u0441\u0442\u043D\u043E"));
  }
}

// src/ui/channels.ts
function isVoice(c) {
  return c.kind === 2 || c.kind === 13;
}
async function loadChannels(guild) {
  const channelsEl = el("channels");
  state.currentGuildId = guild.id;
  state.voiceMemberEls = {};
  channelsEl.innerHTML = "";
  const cat = document.createElement("div");
  cat.className = "channel-category";
  cat.textContent = guild.name;
  channelsEl.appendChild(cat);
  try {
    const channels = await api.getChannels(guild.id);
    state.channels = channels;
    const textChannel = channels.find((c) => c.kind === 0) ?? channels[0];
    if (textChannel) void loadMembers(guild.id, textChannel.id);
    channels.forEach((c) => {
      const div = document.createElement("div");
      div.className = `channel-item ${isVoice(c) ? "voice" : "text"}`;
      div.dataset.channelId = c.id;
      const nameSpan = document.createElement("span");
      nameSpan.className = "channel-name";
      nameSpan.textContent = c.name;
      const badge = document.createElement("span");
      badge.className = "unread-badge";
      div.appendChild(nameSpan);
      div.appendChild(badge);
      state.channelEls[c.id] = div;
      updateBadge(c.id);
      div.onclick = () => {
        document.querySelectorAll(".channel-item").forEach((node) => node.classList.remove("active"));
        div.classList.add("active");
        state.unread.delete(c.id);
        updateBadge(c.id);
        if (isVoice(c)) void joinVoice(c);
        else void openTextChannel(c);
      };
      channelsEl.appendChild(div);
      if (isVoice(c)) {
        const subs = document.createElement("div");
        subs.className = "voice-members";
        state.voiceMemberEls[c.id] = subs;
        channelsEl.appendChild(subs);
      }
    });
    renderVoiceMembers();
  } catch (e) {
    showError("\u041D\u0435 \u0443\u0434\u0430\u043B\u043E\u0441\u044C \u0437\u0430\u0433\u0440\u0443\u0437\u0438\u0442\u044C \u043A\u0430\u043D\u0430\u043B\u044B: " + e);
  }
}

// src/ui/servers.ts
function serverAvatar(guild) {
  if (guild.icon) {
    return `https://cdn.discordapp.com/icons/${guild.id}/${guild.icon}.png?size=64`;
  }
  return null;
}
async function loadGuilds() {
  const serversEl = el("servers");
  try {
    const guilds = await api.getGuilds();
    serversEl.innerHTML = "";
    if (!guilds || guilds.length === 0) {
      const empty = document.createElement("div");
      empty.className = "empty-servers";
      empty.textContent = "\u0421\u0435\u0440\u0432\u0435\u0440\u044B \u043D\u0435 \u043D\u0430\u0439\u0434\u0435\u043D\u044B";
      serversEl.appendChild(empty);
      return;
    }
    guilds.forEach((g) => {
      const div = document.createElement("div");
      div.className = "server-item";
      div.title = g.name;
      const imgUrl = serverAvatar(g);
      if (imgUrl) {
        const img = document.createElement("img");
        img.src = imgUrl;
        img.alt = g.name;
        img.onerror = () => {
          div.textContent = g.name.charAt(0).toUpperCase();
          div.classList.add("server-letter");
        };
        div.appendChild(img);
      } else {
        div.textContent = g.name.charAt(0).toUpperCase();
        div.classList.add("server-letter");
      }
      div.onclick = () => {
        document.querySelectorAll(".server-item").forEach((node) => node.classList.remove("active"));
        div.classList.add("active");
        void loadChannels(g);
      };
      serversEl.appendChild(div);
    });
  } catch (e) {
    showError("\u041D\u0435 \u0443\u0434\u0430\u043B\u043E\u0441\u044C \u0437\u0430\u0433\u0440\u0443\u0437\u0438\u0442\u044C \u0441\u0435\u0440\u0432\u0435\u0440\u044B: " + e);
  }
}

// src/login.ts
function showApp(username) {
  el("login").style.display = "none";
  el("app").style.display = "flex";
  el("username").textContent = username;
  el("avatar").textContent = username.charAt(0).toUpperCase();
  void loadGuilds();
}
function connectGateway() {
  api.connectGateway().catch((e) => console.warn("gateway:", e));
}
async function tryRestoreSession() {
  try {
    const me = await api.getMe();
    showApp(me.username);
    connectGateway();
  } catch {
  }
}
function initLogin() {
  const loginStatus = el("loginStatus");
  const loginBtn = el("loginBtn");
  loginBtn.addEventListener("click", async () => {
    loginBtn.disabled = true;
    loginStatus.textContent = "\u041E\u0442\u043A\u0440\u044B\u0432\u0430\u044E Discord \u0432 \u0431\u0440\u0430\u0443\u0437\u0435\u0440\u0435\u2026";
    try {
      const res = await api.startLogin();
      showApp(res.username);
      connectGateway();
    } catch (e) {
      loginStatus.textContent = String(e);
      showError(e);
    } finally {
      loginBtn.disabled = false;
    }
  });
  const tokenInput = el("tokenInput");
  const tokenBtn = el("tokenBtn");
  tokenBtn.addEventListener("click", async () => {
    const token = tokenInput.value.trim();
    if (!token) {
      showError("\u0412\u0441\u0442\u0430\u0432\u044C \u0442\u043E\u043A\u0435\u043D \u0432 \u043F\u043E\u043B\u0435");
      return;
    }
    tokenBtn.disabled = true;
    loginStatus.textContent = "\u041F\u0440\u043E\u0432\u0435\u0440\u044F\u044E \u0442\u043E\u043A\u0435\u043D\u2026";
    try {
      const res = await api.loginWithToken(token);
      tokenInput.value = "";
      showApp(res.username);
      connectGateway();
    } catch (e) {
      loginStatus.textContent = String(e);
      showError(e);
    } finally {
      tokenBtn.disabled = false;
    }
  });
}

// src/ui/status.ts
function setGatewayStatus(status, error) {
  const pill = el("appStatus");
  const text = el("statusText");
  switch (status) {
    case "connected":
    case "ready":
      pill.dataset.state = "connected";
      text.textContent = "\u041F\u043E\u0434\u043A\u043B\u044E\u0447\u0435\u043D\u043E \u043A Discord";
      break;
    case "reconnecting":
      pill.dataset.state = "reconnecting";
      text.textContent = error ? `\u041F\u0435\u0440\u0435\u043F\u043E\u0434\u043A\u043B\u044E\u0447\u0435\u043D\u0438\u0435\u2026 (${error})` : "\u041F\u0435\u0440\u0435\u043F\u043E\u0434\u043A\u043B\u044E\u0447\u0435\u043D\u0438\u0435\u2026";
      break;
    default:
      pill.dataset.state = "disconnected";
      text.textContent = "\u041D\u0435\u0442 \u0441\u043E\u0435\u0434\u0438\u043D\u0435\u043D\u0438\u044F";
  }
}

// src/app.ts
onMessageCreate((p) => appendLiveMessage(p));
onGatewayStatus((p) => setGatewayStatus(p.status, p.error));
onVoiceStatus((p) => handleVoiceStatus(p));
onVoiceChannelReady(() => {
  setVoiceStatus("\u0413\u043E\u043B\u043E\u0441: \u0434\u0430\u043D\u043D\u044B\u0435 \u0441\u043E\u0431\u0440\u0430\u043D\u044B, \u043F\u043E\u0434\u043A\u043B\u044E\u0447\u0430\u044E\u0441\u044C \u043A \u0433\u043E\u043B\u043E\u0441\u043E\u0432\u043E\u043C\u0443 \u0441\u0435\u0440\u0432\u0435\u0440\u0443\u2026");
});
onPresenceUpdate((p) => handlePresenceUpdate(p));
onVoiceMemberUpdate((p) => handleVoiceMemberUpdate(p));
initLogin();
initChatInput();
initVoiceControls();
el("logoutBtn").onclick = async () => {
  try {
    await api.logout();
  } catch {
  }
  location.reload();
};
void tryRestoreSession();
