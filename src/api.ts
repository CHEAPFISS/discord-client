// Типизированные обёртки Rust-команд (#[tauri::command]).
// Имена команд и аргументы должны совпадать с бэкендом:
// расхождения ловятся при `npm run fe:check`.

import { invoke } from '@tauri-apps/api/core';

/** Сервер Discord (гильдия). */
export interface Guild {
  id: string;
  name: string;
  icon: string | null;
}

/** Канал сервера. kind: 0 — текст, 2 — голос, 13 — stage (поле `type` из API Discord). */
export interface Channel {
  id: string;
  guild_id: string;
  name: string;
  kind: number;
}

/** Сообщение чата. */
export interface ChatMessage {
  id: string;
  channel_id: string;
  author: string;
  content: string;
  timestamp: string;
}

/** Участник сервера. */
export interface Member {
  user_id: string;
  username: string;
  avatar: string | null;
  bot: boolean;
  roles: string[];
}

/** Группа списка участников: id hoist-роли либо "online"/"offline" (как у Discord). */
export interface MemberGroup {
  id: string;
  members: Member[];
}

/** Роль сервера. */
export interface Role {
  id: string;
  name: string;
  color: number;
  position: number;
  hoist: boolean;
}

/** Presence-статусы и голосовые состояния участников. */
export interface GuildActivity {
  presences: Record<string, string>;
  voice: Record<string, string>;
}

/** Текущий пользователь. */
export interface Me {
  user_id: string;
  username: string;
}

export const api = {
  // --- Вход ---
  getMe: () => invoke<Me>('get_me'),
  startLogin: () => invoke<Me>('start_login'),
  loginWithToken: (token: string) => invoke<Me>('login_with_token', { token }),
  logout: () => invoke<void>('logout'),

  // --- Gateway ---
  connectGateway: () => invoke<string>('connect_gateway'),

  // --- REST Discord ---
  getGuilds: () => invoke<Guild[]>('get_guilds'),
  getChannels: (guildId: string) => invoke<Channel[]>('get_channels', { guildId }),
  getGuildMembers: (guildId: string, channelId: string) =>
    invoke<MemberGroup[]>('get_guild_members', { guildId, channelId }),
  getGuildRoles: (guildId: string) => invoke<Role[]>('get_guild_roles', { guildId }),
  getGuildActivity: (guildId: string) => invoke<GuildActivity>('get_guild_activity', { guildId }),
  getMessages: (channelId: string) => invoke<ChatMessage[]>('get_messages', { channelId }),
  sendMessage: (channelId: string, content: string) =>
    invoke<ChatMessage>('send_message', { channelId, content }),

  // --- Голос ---
  joinVoice: (channelId: string, guildId: string) =>
    invoke<void>('join_voice_channel', { channelId, guildId }),
  leaveVoice: () => invoke<void>('leave_voice_channel'),
  setSelfMute: (muted: boolean) => invoke<void>('set_self_mute', { muted }),
  setSelfDeaf: (deaf: boolean) => invoke<void>('set_self_deaf', { deaf }),
};
