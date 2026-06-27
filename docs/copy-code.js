document.addEventListener('DOMContentLoaded', () => {
  document.querySelectorAll('pre').forEach((pre) => {
    const code = pre.querySelector('code');
    if (!code) return;

    const wrapper = document.createElement('div');
    wrapper.className = 'code-block-wrapper';
    pre.parentNode.insertBefore(wrapper, pre);
    wrapper.appendChild(pre);

    const button = document.createElement('button');
    button.className = 'copy-code-button';
    button.type = 'button';
    button.title = 'Copy';
    button.setAttribute('aria-label', 'Copy code');
    button.innerHTML = '<span class="copy-icon">📋</span>';
    wrapper.appendChild(button);

    button.addEventListener('click', async () => {
      try {
        await navigator.clipboard.writeText(code.innerText);
      } catch (err) {
        // Fallback for older browsers or denied permission.
        const textarea = document.createElement('textarea');
        textarea.value = code.innerText;
        textarea.style.position = 'fixed';
        textarea.style.opacity = '0';
        document.body.appendChild(textarea);
        textarea.select();
        document.execCommand('copy');
        document.body.removeChild(textarea);
      }

      button.innerHTML = '<span class="copy-icon copy-success">✓</span>';
      setTimeout(() => {
        button.innerHTML = '<span class="copy-icon">📋</span>';
      }, 1500);
    });
  });
});
