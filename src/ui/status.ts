// Верхняя строка состояния (gateway-соединение).

import { el } from '../dom';

export function setGatewayStatus(status: string, error?: string): void {
  const dot = el('statusDot');
  const text = el('statusText');
  dot.className = 'status-dot';
  switch (status) {
    case 'connected':
    case 'ready':
      dot.classList.add('connected');
      text.textContent = 'Подключено к Discord';
      text.style.color = '#dcddde';
      break;
    case 'reconnecting':
      dot.classList.add('connecting');
      text.textContent = error ? `Переподключение… (${error})` : 'Переподключение…';
      text.style.color = '#faa61a';
      break;
    default:
      dot.classList.add('disconnected');
      text.textContent = 'Нет соединения';
      text.style.color = '#8e9297';
  }
}
