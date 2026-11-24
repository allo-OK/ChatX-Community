if(key){
  conversationProfileCache.set(key,p||{});
  const same=name=>String(name||'').trim().toLowerCase()===key;
  for(const users of [onlineUsers,editorDirectory])for(const user of users)if(same(user.name)){
    user.displayName=p.displayName||p.name;user.avatarUrl=p.avatarUrl||null;
  }
  for(const message of messageStore)if(same(message.sender))message.senderAvatar=p.avatarUrl||null;
  const messages=new Map(messageStore.map(message=>[String(message.id),message]));
  document.querySelectorAll('.msg-row[data-message-id]').forEach(row=>{
    const message=messages.get(row.dataset.messageId);
    if(message&&same(message.sender)){
      row._renderContentKey=null;
      const avatar=row.querySelector(':scope > .message-avatar');
      if(avatar)avatar.outerHTML=avatarHtml(p.avatarUrl,p.name);
    }
    if(message?.reply&&same(message.reply.sender)){
      const label=row.querySelector('.reply-quote b');
      if(label)label.textContent=profileDisplayName(message.reply.sender);
    }
  });
  updateMentionPicker();
}
