// Список серверов (левая колонка).

import { api, type Guild } from '../api';
import { el } from '../dom';
import { loadChannels } from './channels';
import { showError } from './toast';

function serverAvatar(guild: Guild): string | null {
  if (guild.icon) {
    return `https://cdn.discordapp.com/icons/${guild.id}/${guild.icon}.png?size=64`;
  }
  return null;
}

export async function loadGuilds(): Promise<void> {
  const serversEl = el('servers');
  try {
    const guilds = await api.getGuilds();
    serversEl.innerHTML = '';
    if (!guilds || guilds.length === 0) {
      const empty = document.createElement('div');
      empty.className = 'empty-servers';
      empty.textContent = 'Серверы не найдены';
      serversEl.appendChild(empty);
      return;
    }
    guilds.forEach((g) => {
      const div = document.createElement('div');
      div.className = 'server-item';
      div.title = g.name;
      const imgUrl = serverAvatar(g);
      if (imgUrl) {
        const img = document.createElement('img');
        img.src = imgUrl;
        img.alt = g.name;
        img.onerror = () => {
          div.textContent = g.name.charAt(0).toUpperCase();
          div.classList.add('server-letter');
        };
        div.appendChild(img);
      } else {
        div.textContent = g.name.charAt(0).toUpperCase();
        div.classList.add('server-letter');
      }
      div.onclick = () => {
        document
          .querySelectorAll('.server-item')
          .forEach((node) => node.classList.remove('active'));
        div.classList.add('active');
        void loadChannels(g);
      };
      serversEl.appendChild(div);
    });
  } catch (e) {
    showError('Не удалось загрузить серверы: ' + e);
  }
}
