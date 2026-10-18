// Клиентский стейт (единственный мутабельный объект на всё приложение).

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
};
