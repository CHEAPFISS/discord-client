// Типизированные подписки на события бэкенда (app.emit в Rust).

import { listen } from '@tauri-apps/api/event';
import type { ActivityInfo, ChatMessage } from './api';

/** MESSAGE_CREATE из Gateway (текстовый канал): формат совпадает с историей REST. */
export type MessageCreatePayload = ChatMessage;

/** Состояние gateway-соединения. */
export interface GatewayStatusPayload {
  status: 'connected' | 'ready' | 'reconnecting' | 'disconnected';
  error?: string | null;
}

/** Состояние голосового соединения. */
export interface VoiceStatusPayload {
  status:
    | 'left'
    | 'joining'
    | 'connected'
    | 'reconnecting'
    | 'connecting'
    | 'needs_fresh'
    | 'error';
  error?: string;
}

/** Голосовая пара собрана — бэкенд подключается к голосовому серверу сам. */
export interface VoiceChannelReadyPayload {
  guild_id: string;
  channel_id: string;
  session_id: string;
  endpoint: string;
}

/** PRESENCE_UPDATE: изменился статус участника (online/idle/dnd/offline)
 *  и/или его активности (Rich Presence). */
export interface PresenceUpdatePayload {
  guild_id: string;
  user_id: string;
  status: string;
  activities: ActivityInfo[];
}

/** VOICE_STATE_UPDATE: участник вошёл/вышел из голосового канала. */
export interface VoiceMemberUpdatePayload {
  guild_id: string;
  user_id: string;
  channel_id: string | null;
}

/** VOICE_SPEAKING: индикатор «говорит» (по аудио-кадрам; false — замолчал). */
export interface VoiceSpeakingPayload {
  user_id: string;
  speaking: boolean;
}

function subscribe<T>(event: string, cb: (p: T) => void): void {
  listen<T>(event, (e) => cb(e.payload)).catch((err) => {
    console.warn(`listen(${event}):`, err);
  });
}

export const onMessageCreate = (cb: (p: MessageCreatePayload) => void): void =>
  subscribe('message_create', cb);

export const onGatewayStatus = (cb: (p: GatewayStatusPayload) => void): void =>
  subscribe('gateway_status', cb);

export const onVoiceStatus = (cb: (p: VoiceStatusPayload) => void): void =>
  subscribe('voice_status', cb);

export const onVoiceChannelReady = (cb: (p: VoiceChannelReadyPayload) => void): void =>
  subscribe('voice_channel_ready', cb);

export const onPresenceUpdate = (cb: (p: PresenceUpdatePayload) => void): void =>
  subscribe('presence_update', cb);

export const onVoiceMemberUpdate = (cb: (p: VoiceMemberUpdatePayload) => void): void =>
  subscribe('voice_member_update', cb);

export const onVoiceSpeaking = (cb: (p: VoiceSpeakingPayload) => void): void =>
  subscribe('voice_speaking', cb);
