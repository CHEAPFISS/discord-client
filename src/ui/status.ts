// Верхняя строка состояния (gateway-соединение).
// Цвета — целиком в CSS (main.css), здесь только состояние и текст.

import { el } from '../dom';

export function setGatewayStatus(status: string, error?: string): void {
  const pill = el('appStatus');
  const text = el('statusText');
  switch (status) {
    case 'connected':
    case 'ready':
      pill.dataset.state = 'connected';
      text.textContent = 'Подключено к Discord';
      break;
    case 'reconnecting':
      pill.dataset.state = 'reconnecting';
      text.textContent = error ? `Переподключение… (${error})` : 'Переподключение…';
      break;
    default:
      pill.dataset.state = 'disconnected';
      text.textContent = 'Нет соединения';
  }
}
