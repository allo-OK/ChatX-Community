(() => {
  'use strict';
  for (const id of ['weightCalculator', 'shippingCalculator', 'adminModal', 'clientUpdateModal', 'notificationModal', 'storageModal', 'systemModal', 'diagnosticsModal']) {
    const card = document.querySelector('#' + id + ' :is(.modal-card, .weight-card)');
    const head = card?.querySelector(':scope > .modal-head');
    if (!head || card.querySelector(':scope > .kc-modal-content')) continue;
    const content = document.createElement('div');
    content.className = 'kc-modal-content';
    content.tabIndex = -1;
    for (const node of [...card.childNodes]) {
      if (node !== head) content.append(node);
    }
    head.after(content);
    card.classList.add('kc-fixed-header');
  }
})();
