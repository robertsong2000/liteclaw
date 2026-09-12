// Source attachments use authenticated fetches, never credentials in URLs.
function renderSourceImages(images, answer) {
  const references = Array.isArray(images)
    ? images.filter(item => item && /^[a-zA-Z0-9_-]+$/.test(item.id || '')).slice(0, 6)
    : [];
  const signature = JSON.stringify(references);
  if (answer.dataset.sourceImages === signature) return;
  answer.dataset.sourceImages = signature;
  // Repeated streaming renders replace attachments without duplicating cards.
  answer.querySelectorAll(':scope > .manual-gallery, :scope > .manual-references').forEach(el => el.remove());
  if (!references.length) return;
  const featured = references.filter(item => item.role === 'primary' || item.role === 'supporting').slice(0, 3);
  const sources = references.filter(item => !featured.includes(item));
  if (featured.length) renderImageGroup(featured, answer, true);
  if (sources.length) {
    const details = document.createElement('details');
    details.className = 'manual-references';
    const summary = document.createElement('summary');
    summary.textContent = '查看手册原文参考 / Source pages';
    details.append(summary);
    answer.append(details);
    details.addEventListener('toggle', () => {
      if (details.open && !details.querySelector('.manual-gallery')) renderImageGroup(sources, details, false);
    });
  }
}

function renderImageGroup(references, answer, featured) {
  const gallery = document.createElement('section');
  gallery.className = 'manual-gallery' + (featured ? ' manual-primary' : '');
  gallery.setAttribute('aria-label', '手册图片 / Manual images');
  const heading = document.createElement('h4');
  heading.textContent = featured ? '相关操作图 · 点击放大 / Selected illustrations' : '手册原图 / Source pages';
  gallery.append(heading);
  for (const item of references) {
    const card = document.createElement('figure');
    const button = document.createElement('button');
    button.type = 'button';
    button.textContent = '加载图片…';
    const caption = document.createElement('figcaption');
    caption.textContent = `PDF p.${item.page} · ${item.kind === 'source_page' ? '原文页面' : '相关插图'} — ${item.caption || ''}`;
    card.append(button, caption);
    gallery.append(card);
    fetch('/api/manual-images/' + encodeURIComponent(item.id), {headers: authHeaders()})
      .then(r => { if (!r.ok) throw Error(String(r.status)); return r.blob(); })
      .then(blob => {
        const reader = new FileReader();
        reader.onload = () => {
          const img = document.createElement('img');
          img.src = reader.result;
          img.alt = item.caption || `手册 PDF 第 ${item.page} 页`;
          button.replaceChildren(img);
          button.setAttribute('aria-label', `放大查看 PDF 第 ${item.page} 页`);
          button.onclick = () => {
            const dialog = document.createElement('dialog');
            dialog.className = 'manual-image-dialog';
            const close = document.createElement('button');
            close.textContent = '关闭 / Close';
            const full = img.cloneNode();
            const label = document.createElement('p');
            label.textContent = caption.textContent;
            dialog.append(close, full, label);
            document.body.append(dialog);
            close.onclick = () => dialog.close();
            dialog.addEventListener('close', () => { dialog.remove(); button.focus(); });
            dialog.onclick = e => { if (e.target === dialog) dialog.close(); };
            dialog.showModal();
            close.focus();
          };
        };
        reader.readAsDataURL(blob);
      })
      .catch(() => { button.textContent = '图片暂不可用，请重新登录后重试'; button.disabled = true; });
  }
  if (featured) answer.prepend(gallery);
  else answer.append(gallery);
}
