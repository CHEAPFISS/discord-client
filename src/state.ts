// Клиентский стейт (единственный мутабельный объект на всё приложение).

import type { Channel, Member, MemberGroup, PresenceInfo, Role, VoiceStateInfo } from './api';

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
  /** user_id участников, которые сейчас говорят (зелёная обводка в мини-списке). */
  voiceSpeaking: new Set<string>(),
  /** channelId → количество непрочитанных. */
  unread: new Map<string, number>(),
  /** channelId → DOM-элемент канала (для бейджей). */
  channelEls: {} as Record<string, HTMLElement>,

  // --- Участники сервера (правая колонка) ---
  /** Каналы текущего сервера (для имён голосовых каналов в списке). */
  channels: [] as Channel[],
  /** Участники текущего сервера. */
  members: [] as Member[],
  /** Группы участников в порядке отображения Discord (hoist-роли + online/offline). */
  memberGroups: [] as MemberGroup[],
  /** Роли текущего сервера (для группировки). */
  roles: [] as Role[],
  /** user_id → presence-статус (online/idle/dnd/offline). */
  presences: new Map<string, PresenceInfo>(),
  /** user_id, чей presence обновился живым PRESENCE_UPDATE при загрузке —
   *  снапшот из get_guild_activity их не должен затирать (свежее у живого). */
  presencesTouched: new Set<string>(),
  /** user_id → голосовое состояние (канал + флаги демонстрации экрана/камеры). */
  memberVoice: new Map<string, VoiceStateInfo>(),
  /** channelId → контейнер под голосовым каналом (мини-список участников). */
  voiceMemberEls: {} as Record<string, HTMLElement>,
  /** Сервер, для которого сейчас загружены участники. */
  membersGuildId: null as string | null,
  /** Идёт загрузка участников (не перерисовываем промежуточно). */
  membersLoading: false,

  // --- Чужая демонстрация экрана (focus-режим) ---
  /** Что смотрим прямо сейчас: ключ стрима + владелец (null — ничего). */
  watching: null as { stream_key: string; owner_id: string; username: string } | null,
};
