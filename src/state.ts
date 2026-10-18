// Клиентский стейт (единственный мутабельный объект на всё приложение).

import type { Channel, Member, Role } from './api';

export const state = {
  /** Открытый сервер (нужен для join_voice_channel). */
  currentGuildId: null as string | null,
  /** Открытый текстовый канал. */
  currentChannelId: null as string | null,
  /** Активный голосовой канал (запоминаем для UI). */
  currentVoiceChannelId: null as string | null,
  /** Локальные флаги кнопок голосовой панели. */
  micMuted: false,
  deafened: false,
  /** channelId → количество непрочитанных. */
  unread: new Map<string, number>(),
  /** channelId → DOM-элемент канала (для бейджей). */
  channelEls: {} as Record<string, HTMLElement>,

  // --- Участники сервера (правая колонка) ---
  /** Каналы текущего сервера (для имён голосовых каналов в списке). */
  channels: [] as Channel[],
  /** Участники текущего сервера. */
  members: [] as Member[],
  /** Роли текущего сервера (для группировки). */
  roles: [] as Role[],
  /** user_id → presence-статус (online/idle/dnd/offline). */
  presences: new Map<string, string>(),
  /** user_id → channel_id (кто в каком голосовом канале). */
  memberVoice: new Map<string, string>(),
  /** Сервер, для которого сейчас загружены участники. */
  membersGuildId: null as string | null,
  /** Идёт загрузка участников (не перерисовываем промежуточно). */
  membersLoading: false,
};
