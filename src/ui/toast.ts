// Всплывающее уведомление об ошибке.

export function showError(msg: unknown): void {
  const text = String(msg);
  const toast = document.getElementById('errorToast');
  if (!toast) {
    alert(text);
    return;
  }
  toast.textContent = text;
  toast.style.display = 'block';
  setTimeout(() => {
    toast.style.display = 'none';
  }, 8000);
  toast.onclick = () => {
    toast.style.display = 'none';
  };
}
