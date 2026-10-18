// Экран входа: OAuth, вход по токену, автовосстановление сессии.

import { api } from './api';
import { el } from './dom';
import { loadGuilds } from './ui/servers';
import { showError } from './ui/toast';

/** Переключение на основной экран после успешного входа. */
export function showApp(username: string): void {
  el('login').style.display = 'none';
  el('app').style.display = 'flex';
  el('username').textContent = username;
  el('userAreaName').textContent = username;
  el('avatar').textContent = username.charAt(0).toUpperCase();
  void loadGuilds();
}

function connectGateway(): void {
  api.connectGateway().catch((e) => console.warn('gateway:', e));
}

/** Автовосстановление сессии: если токен сохранён — сразу в приложение. */
export async function tryRestoreSession(): Promise<void> {
  try {
    const me = await api.getMe();
    showApp(me.username);
    connectGateway();
  } catch {
    // не залогинен — остаёмся на экране входа
  }
}

/** Обработчики экрана входа: OAuth-кнопка и вход по user-токену. */
export function initLogin(): void {
  const loginStatus = el('loginStatus');
  const loginBtn = el<HTMLButtonElement>('loginBtn');

  loginBtn.addEventListener('click', async () => {
    loginBtn.disabled = true;
    loginStatus.textContent = 'Открываю Discord в браузере…';
    try {
      const res = await api.startLogin();
      showApp(res.username);
      connectGateway();
    } catch (e) {
      loginStatus.textContent = String(e);
      showError(e);
    } finally {
      loginBtn.disabled = false;
    }
  });

  const tokenInput = el<HTMLInputElement>('tokenInput');
  const tokenBtn = el<HTMLButtonElement>('tokenBtn');

  tokenBtn.addEventListener('click', async () => {
    const token = tokenInput.value.trim();
    if (!token) {
      showError('Вставь токен в поле');
      return;
    }
    tokenBtn.disabled = true;
    loginStatus.textContent = 'Проверяю токен…';
    try {
      const res = await api.loginWithToken(token);
      tokenInput.value = '';
      showApp(res.username);
      connectGateway();
    } catch (e) {
      loginStatus.textContent = String(e);
      showError(e);
    } finally {
      tokenBtn.disabled = false;
    }
  });
}
