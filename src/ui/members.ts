// Колонка участников сервера: группы и порядок — как их прислал Discord
// (hoist-роли + «В сети»/«Не в сети»), presence-статусы, бейдж «БОТ»,
// «🔊 В голосовом чате» — только если никакого статуса нет.

import { api, type Member } from '../api';
import { el } from '../dom';
import type { PresenceUpdatePayload, VoiceMemberUpdatePayload } from '../events';
import { state } from '../state';

/** URL аватарки с CDN Discord (animated → .gif). */
function avatarUrl(m: Member): string | null {
  if (!m.avatar) return null;
  const ext = m.avatar.startsWith('a_') ? 'gif' : 'png';
  return `https://cdn.discordapp.com/avatars/${m.user_id}/${m.avatar}.${ext}?size=64`;
}

function statusOf(userId: string): string {
  return state.presences.get(userId)?.status ?? 'offline';
}

/** Иконки-маркеры по типу активности (как в списке участников Discord:
 * игра — геймпад, музыка — нота, …). Официальной таблицы нет — Discord
 * рисует свои глифы; ориентир — их клиент. */
const ACTIVITY_ICONS: Record<number, string> = {
  0: '🎮', // игра
  1: '📡', // стрим
  2: '🎵', // музыка/прослушивание
  3: '📺', // просмотр
  5: '🏆', // соревнование
};

/** Строка статуса под ником — одна, как в списке участников Discord:
 * кастом-статус перебивает остальные активности («Custom statuses override
 * other activity statuses in a server's members list»); если кастома нет —
 * первая активность в том порядке, как её прислал Discord. */
function activityLines(userId: string): { text: string; title?: string }[] {
  const p = state.presences.get(userId);
  if (!p || p.status === 'offline') return [];

  const custom = p.activities.find((a) => a.kind === 4);
  const others = p.activities.filter((a) => a.kind !== 4 && a.name);
  if (custom) {
    const text = [custom.emoji, custom.state].filter(Boolean).join(' ').trim();
    if (text) {
      // Активности, скрытые за кастом-статусом, Discord помечает «иконка+N •».
      const mark =
        others.length > 0
          ? `${ACTIVITY_ICONS[others[0].kind] ?? '🎮'}+${others.length} • `
          : '';
      return [{ text: mark + text }];
    }
  }

  const act = others[0];
  if (!act) return [];
  const prefixes: Record<number, string> = {
    0: 'Играет в',
    1: 'Стримит',
    2: 'Слушает',
    3: 'Смотрит',
    5: 'Соревнуется в',
  };
  const extra = [act.details, act.state].filter(Boolean).join(' — ');
  return [
    {
      text: `${ACTIVITY_ICONS[act.kind] ?? '🎮'} ${prefixes[act.kind] ?? 'Играет в'} ${act.name}`,
      title: extra ? `${act.name} — ${extra}` : undefined,
    },
  ];
}

function roleColor(color: number): string | null {
  return color ? '#' + color.toString(16).padStart(6, '0') : null;
}

/** Загрузка участников/ролей/активности при выборе сервера. */
export async function loadMembers(guildId: string, channelId: string): Promise<void> {
  state.membersGuildId = guildId;
  state.membersLoading = true;
  state.members = [];
  state.memberGroups = [];
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
    const groups = await api.getGuildMembers(guildId, channelId);
    // Пока грузились — могли переключить сервер.
    if (state.membersGuildId !== guildId) return;
    const [roles, activity] = await Promise.all([
      api.getGuildRoles(guildId),
      api.getGuildActivity(guildId),
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
    membersEl.innerHTML = '';
    const err = document.createElement('div');
    err.className = 'members-empty';
    err.textContent = 'Не удалось загрузить участников: ' + e;
    membersEl.appendChild(err);
  }
}

/** PRESENCE_UPDATE: обновляем статус и активности участника. */
export function handlePresenceUpdate(p: PresenceUpdatePayload): void {
  if (p.guild_id !== state.membersGuildId) return;
  state.presences.set(p.user_id, { status: p.status, activities: p.activities });
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

  const groups: Group[] = [];

  // Группы и их порядок — прямо из списка Discord (hoist-роли по иерархии,
  // затем «online»/«offline»). Внутри групп порядок Discord тоже сохраняем.
  const roleById = new Map(state.roles.map((r) => [r.id, r]));
  const rest: Member[] = [];
  for (const g of state.memberGroups) {
    if (g.id === 'online' || g.id === 'offline') {
      rest.push(...g.members);
      continue;
    }
    if (g.members.length === 0) continue;
    const role = roleById.get(g.id);
    groups.push({
      name: role?.name ?? 'Участники',
      color: role ? roleColor(role.color) : null,
      members: g.members,
    });
  }
  // «В сети»/«Не в сети» — по живым статусам (участник с hoist-ролью
  // остаётся в своей роли, как в официальном клиенте).
  const online = rest.filter((m) => statusOf(m.user_id) !== 'offline');
  const offline = rest.filter((m) => statusOf(m.user_id) === 'offline');
  if (online.length > 0) groups.push({ name: 'В сети', color: null, members: online });
  if (offline.length > 0) groups.push({ name: 'Не в сети', color: null, members: offline });

  for (const g of groups) {
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

  // Статус — одна строка: кастом-статус либо активность (как в Discord).
  // Голосовая строка — только фолбэк, если статуса нет вовсе (в официальном
  // клиенте у людей со статусом «в голосовом чате» не выводится).
  const lines = activityLines(m.user_id);
  if (lines.length > 0) {
    for (const line of lines) {
      const sub = document.createElement('div');
      sub.className = 'member-sub';
      sub.textContent = line.text;
      if (line.title) sub.title = line.title;
      body.appendChild(sub);
    }
  } else if (state.memberVoice.has(m.user_id)) {
    const sub = document.createElement('div');
    sub.className = 'member-sub';
    sub.textContent = '🔊 В голосовом чате';
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
