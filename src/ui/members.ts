// Колонка участников сервера: группировка по ролям (как в Discord),
// presence-статусы, бейдж «БОТ», пометка «в голосовом канале».

import { api, type Member, type Role } from '../api';
import { el } from '../dom';
import type { PresenceUpdatePayload, VoiceMemberUpdatePayload } from '../events';
import { state } from '../state';

/** Порядок статусов для сортировки (как в Discord). */
const STATUS_ORDER: Record<string, number> = { online: 0, idle: 1, dnd: 2, offline: 3 };

/** URL аватарки с CDN Discord (animated → .gif). */
function avatarUrl(m: Member): string | null {
  if (!m.avatar) return null;
  const ext = m.avatar.startsWith('a_') ? 'gif' : 'png';
  return `https://cdn.discordapp.com/avatars/${m.user_id}/${m.avatar}.${ext}?size=64`;
}

function statusOf(userId: string): string {
  return state.presences.get(userId) ?? 'offline';
}

function roleColor(color: number): string | null {
  return color ? '#' + color.toString(16).padStart(6, '0') : null;
}

/** Загрузка участников/ролей/активности при выборе сервера. */
export async function loadMembers(guildId: string, channelId: string): Promise<void> {
  state.membersGuildId = guildId;
  state.membersLoading = true;
  state.members = [];
  state.roles = [];
  const membersEl = el('members');
  membersEl.classList.add('active');
  membersEl.innerHTML = '';
  const loading = document.createElement('div');
  loading.className = 'members-empty';
  loading.textContent = 'Загружаю участников…';
  membersEl.appendChild(loading);

  try {
    // Список участников приходит через gateway (op 37); он же засеял
    // свежие presence-статусы — активность читаем ПОСЛЕ него.
    const members = await api.getGuildMembers(guildId, channelId);
    // Пока грузились — могли переключить сервер.
    if (state.membersGuildId !== guildId) return;
    const [roles, activity] = await Promise.all([
      api.getGuildRoles(guildId),
      api.getGuildActivity(guildId),
    ]);
    if (state.membersGuildId !== guildId) return;
    state.members = members;
    state.roles = roles;
    state.presences = new Map(Object.entries(activity.presences));
    state.memberVoice = new Map(Object.entries(activity.voice));
    state.membersLoading = false;
    renderMembers();
    renderVoiceMembers();
  } catch (e) {
    if (state.membersGuildId !== guildId) return;
    state.membersLoading = false;
    membersEl.innerHTML = '';
    const err = document.createElement('div');
    err.className = 'members-empty';
    err.textContent = 'Не удалось загрузить участников: ' + e;
    membersEl.appendChild(err);
  }
}

/** PRESENCE_UPDATE: обновляем статус участника. */
export function handlePresenceUpdate(p: PresenceUpdatePayload): void {
  if (p.guild_id !== state.membersGuildId) return;
  state.presences.set(p.user_id, p.status);
  if (!state.membersLoading) renderMembers();
}

/** VOICE_STATE_UPDATE: обновляем «кто в каком голосовом канале». */
export function handleVoiceMemberUpdate(p: VoiceMemberUpdatePayload): void {
  if (p.guild_id !== state.membersGuildId) return;
  if (p.channel_id) state.memberVoice.set(p.user_id, p.channel_id);
  else state.memberVoice.delete(p.user_id);
  if (!state.membersLoading) {
    renderMembers();
    renderVoiceMembers();
  }
}

interface Group {
  name: string;
  color: string | null;
  members: Member[];
}

function renderMembers(): void {
  const membersEl = el('members');
  membersEl.innerHTML = '';
  if (state.members.length === 0) {
    const empty = document.createElement('div');
    empty.className = 'members-empty';
    empty.textContent = 'Нет участников';
    membersEl.appendChild(empty);
    return;
  }

  // Группы = hoist-роли (по position, сверху вниз) + «В сети» / «Не в сети».
  const roleById = new Map(state.roles.map((r) => [r.id, r]));
  const hoisted = state.roles
    .filter((r) => r.hoist && r.position > 0)
    .sort((a, b) => b.position - a.position);

  const buckets = new Map<string, Member[]>(); // role_id → участники
  const rest: Member[] = [];
  for (const m of state.members) {
    let top: Role | null = null;
    for (const rid of m.roles) {
      const r = roleById.get(rid);
      if (r && (!top || r.position > top.position)) top = r;
    }
    if (top && top.hoist && top.position > 0) {
      let arr = buckets.get(top.id);
      if (!arr) {
        arr = [];
        buckets.set(top.id, arr);
      }
      arr.push(m);
    } else {
      rest.push(m);
    }
  }

  const groups: Group[] = [];
  for (const r of hoisted) {
    const ms = buckets.get(r.id);
    if (ms && ms.length > 0) {
      groups.push({ name: r.name, color: roleColor(r.color), members: ms });
    }
  }
  const online = rest.filter((m) => statusOf(m.user_id) !== 'offline');
  const offline = rest.filter((m) => statusOf(m.user_id) === 'offline');
  if (online.length > 0) groups.push({ name: 'В сети', color: null, members: online });
  if (offline.length > 0) groups.push({ name: 'Не в сети', color: null, members: offline });

  for (const g of groups) {
    g.members.sort((a, b) => {
      const sa = STATUS_ORDER[statusOf(a.user_id)] ?? 3;
      const sb = STATUS_ORDER[statusOf(b.user_id)] ?? 3;
      if (sa !== sb) return sa - sb;
      return a.username.localeCompare(b.username, 'ru');
    });

    const groupEl = document.createElement('div');
    groupEl.className = 'member-group';
    const title = document.createElement('div');
    title.className = 'member-group-title';
    title.textContent = `${g.name} — ${g.members.length}`;
    if (g.color) title.style.color = g.color;
    groupEl.appendChild(title);
    for (const m of g.members) groupEl.appendChild(memberRow(m));
    membersEl.appendChild(groupEl);
  }
}

function memberRow(m: Member): HTMLElement {
  const row = document.createElement('div');
  row.className = 'member';

  // Аватарка (или буква, если её нет / не загрузилась)
  const av = document.createElement('div');
  av.className = 'member-avatar';
  const url = avatarUrl(m);
  if (url) {
    const img = document.createElement('img');
    img.src = url;
    img.alt = '';
    img.onerror = () => {
      img.remove();
      av.prepend(letterSpan(m.username));
    };
    av.appendChild(img);
  } else {
    av.appendChild(letterSpan(m.username));
  }
  const dot = document.createElement('span');
  dot.className = `presence ${statusOf(m.user_id)}`;
  av.appendChild(dot);

  // Имя + бейджи
  const body = document.createElement('div');
  body.className = 'member-body';
  const name = document.createElement('div');
  name.className = 'member-name';
  const nameText = document.createElement('span');
  nameText.textContent = m.username;
  name.appendChild(nameText);
  if (m.bot) {
    const bot = document.createElement('span');
    bot.className = 'bot-badge';
    bot.textContent = 'БОТ';
    name.appendChild(bot);
  }
  body.appendChild(name);

  // «В голосовом канале» (если сидит в одном из голосовых)
  const voiceChannel = state.memberVoice.get(m.user_id);
  if (voiceChannel) {
    const sub = document.createElement('div');
    sub.className = 'member-sub';
    const chName = state.channels.find((c) => c.id === voiceChannel)?.name;
    sub.textContent = `🔊 ${chName ?? 'голосовой канал'}`;
    body.appendChild(sub);
  }

  row.appendChild(av);
  row.appendChild(body);
  return row;
}

function letterSpan(name: string, cls = 'member-letter'): HTMLElement {
  const span = document.createElement('span');
  span.className = cls;
  span.textContent = name.charAt(0).toUpperCase();
  return span;
}

/** Мини-список участников под голосовым каналом (как в Discord). */
export function renderVoiceMembers(): void {
  for (const [channelId, container] of Object.entries(state.voiceMemberEls)) {
    container.innerHTML = '';
    const rows = [...state.memberVoice.entries()]
      .filter(([, cid]) => cid === channelId)
      .map(([uid]) => state.members.find((m) => m.user_id === uid))
      .filter((m): m is Member => m !== undefined)
      .sort((a, b) => a.username.localeCompare(b.username, 'ru'));
    for (const m of rows) container.appendChild(voiceMemberRow(m));
  }
}

function voiceMemberRow(m: Member): HTMLElement {
  const row = document.createElement('div');
  row.className = 'voice-member';

  const av = document.createElement('div');
  av.className = 'voice-member-avatar';
  const url = avatarUrl(m);
  if (url) {
    const img = document.createElement('img');
    img.src = url;
    img.alt = '';
    img.onerror = () => {
      img.remove();
      av.prepend(letterSpan(m.username, 'voice-member-letter'));
    };
    av.appendChild(img);
  } else {
    av.appendChild(letterSpan(m.username, 'voice-member-letter'));
  }
  const dot = document.createElement('span');
  dot.className = `presence ${statusOf(m.user_id)}`;
  av.appendChild(dot);

  const name = document.createElement('span');
  name.className = 'voice-member-name';
  name.textContent = m.username;

  row.appendChild(av);
  row.appendChild(name);
  return row;
}
