// Run with NODE_PATH pointing to an existing Playwright installation.
// Exercises the actual streaming renderer with a deterministic SSE boundary.
const { chromium } = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');

(async () => {
  const browser = await chromium.launch({headless: true});
  try {
    const page = await browser.newPage();
    const base = process.env.LITECLAW_URL || 'http://127.0.0.1:9999';
    await page.goto(base);
    await page.waitForLoadState('networkidle');
    console.log('Controls:', await page.locator('input, textarea, select').evaluateAll(es => es.map(e => e.id)));
    const authPath = os.homedir() + '/.liteclaw/auth.json';
    const creds = fs.existsSync(authPath) ? JSON.parse(fs.readFileSync(authPath)) : {username:'renault',password:'renault123'};
    const login = await page.request.post(base+'/api/login', {data:creds});
    assert.equal(login.status(), 200);
    const {token} = await login.json();
    let primaryRequests = 0;
    let referenceRequests = 0;
    page.on('request', request=>{if(request.url().endsWith('/api/manual-images/6869a21848e1c3f0-figure-1-350')) primaryRequests++;});
    page.on('request', request=>{if(request.url().endsWith('/api/manual-images/6869a21848e1c3f0-page-350')) referenceRequests++;});
    await page.evaluate(token => {authToken = token; hideLogin();}, token);
    const attachment = {id:'6869a21848e1c3f0-figure-1-350',page:350,kind:'illustration',role:'primary',caption:'保险丝盒原图，编号 1–12'};
    const reference = {...attachment,id:'6869a21848e1c3f0-page-350',kind:'source_page',role:'reference'};
    let sent;
    await page.route('**/api/history', async route => {
      await route.fulfill({json:{sessions:[]}});
    });
    await page.route('**/api/chat', async route => {
      sent = route.request().postDataJSON();
      const stale = Array.from({length:6},(_,i)=>({id:'stale-'+i,page:i,kind:'source_page',caption:'Earlier search'}));
      const events = [{type:'source_images',images:stale}, {type:'source_images',images:[attachment,reference]},
        {type:'text_delta',text:'我再核对一下。'}, {type:'tool_start',tool:'skill_run',arguments:{id:'manual-rag'}},
        {type:'tool_result',tool:'skill_run',ok:true,summary:'[]'},
        {type:'source_images',images:[attachment,reference]},
        {type:'text_delta',text:'保险丝布局请参见原手册 PDF 第 350 页。'}, {type:'done'}];
      await route.fulfill({contentType:'text/event-stream',body:events.map(e=>'data: '+JSON.stringify(e)+'\n\n').join('')});
    });
    await page.evaluate(async () => {
      window.pendingFrames = [];
      window.requestAnimationFrame = cb => {pendingFrames.push(cb); return pendingFrames.length;};
      currentSessionId = 'manual-image-stream-fixture';
      messages = [{role:'assistant',content:'上一轮',source_images:[{id:'old'}]}, {role:'user',content:'保险丝盒在哪里？'}];
      await streamChat();
      pendingFrames.splice(0).forEach(cb=>cb());
    });
    assert.equal(await page.locator('.manual-gallery').count(), 1, 'final animation frame must not erase source cards');
    assert.ok(await page.locator('.manual-primary').evaluate(el => el === el.parentElement.firstElementChild), 'question-specific main image precedes answer text');
    assert.equal(await page.locator('.manual-references[open]').count(),0,'source pages must be collapsed');
    assert.equal(await page.locator('.manual-references img').count(),0,'collapsed source images load only on demand');
    assert.ok((await page.locator('.manual-gallery figcaption').first().textContent()).includes('PDF p.350'), 'latest retrieval must not be ignored');
    assert.ok(sent.messages.every(m=>!('source_images' in m)), 'attachments must not leak into provider schema');
    await page.waitForFunction(()=>document.querySelector('.manual-gallery img')?.naturalWidth > 0);
    assert.equal(primaryRequests,1,'stream text and tool transitions must reuse the same image load');
    assert.equal(referenceRequests,0,'do not fetch folded original pages');
    const saved = await page.evaluate(()=>histStore().find(s=>s.id==='manual-image-stream-fixture'));
    assert.deepEqual(saved.messages.at(-1).source_images[0], attachment);
    assert.equal(saved.messages.at(-1).source_images.length, 2, 'only selected figure and provenance remain, no stale images');
    await page.locator('.manual-gallery button').first().click();
    assert.equal(await page.locator('dialog[open] img').count(), 1);
    await page.keyboard.press('Escape');
    await page.locator('dialog').waitFor({state:'detached'});
    assert.equal(await page.locator('dialog').count(), 0);
    await page.setViewportSize({width:390,height:844});
    assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth), 'mobile must not overflow');
    await page.locator('.manual-gallery button').first().click();
    await page.screenshot({path:'/tmp/manual-images-mobile-enlarged.png'});
    await page.keyboard.press('Escape');
    await page.reload();
    await page.waitForLoadState('networkidle');
    await page.evaluate(async token=>{
      authToken=token; hideLogin(); currentSessionId=null;
      await switchToSession('manual-image-stream-fixture');
    },token);
    assert.equal(await page.locator('.manual-primary').count(),1, 'reload must restore the question-specific main image');
    assert.equal(await page.locator('.manual-references').count(),2, 'legacy history becomes folded references');
    assert.ok((await page.locator('.manual-gallery').last().innerText()).includes('PDF p.350'));
    await page.locator('.manual-references summary').first().click();
    await page.waitForFunction(()=>document.querySelector('.manual-references .manual-gallery button')?.disabled);
    await page.locator('.manual-references summary').last().click();
    await page.waitForFunction(()=>[...document.querySelectorAll('.manual-references img')].some(i=>i.naturalWidth>0));
    await page.locator('.manual-references summary').last().click();
    await page.locator('.manual-references summary').last().click();
    assert.equal(referenceRequests,1,'reopening original pages reuses loaded images');
    await page.unroute('**/api/chat');
    await page.route('**/api/chat',route=>route.fulfill({contentType:'text/event-stream',body:[
      {type:'source_images',images:[attachment]}, {type:'source_images',images:[]},
      {type:'text_delta',text:'未找到相关配图。'}, {type:'done'}
    ].map(e=>'data: '+JSON.stringify(e)+'\n\n').join('')}));
    await page.evaluate(async()=>{chat.innerHTML='';messages=[{role:'user',content:'没有相关配图'}];await streamChat();});
    assert.equal(await page.locator('.manual-gallery').count(),0,'empty new result must clear earlier main image');
    assert.deepEqual(await page.evaluate(()=>messages.at(-1).source_images),[]);
    await page.evaluate(()=>{
      messages=[{role:'assistant',content:'纯文字回答',source_images:[]}];
      renderChatFromMessages();
    });
    assert.equal(await page.locator('.manual-gallery').count(),0, 'text-only history must not show an empty gallery');
    await page.unroute('**/api/chat');
    await page.route('**/api/chat', route => route.fulfill({
      contentType:'text/event-stream',
      body:'data: '+JSON.stringify({type:'source_images',images:[attachment]})+'\n\n'
    }));
    await page.evaluate(async()=>{
      chat.innerHTML=''; messages=[{role:'user',content:'启动方法'}];
      await streamChat();
    });
    assert.equal(await page.locator('#chat .err').count(), 1,
      'EOF without done or error must not silently leave an image-only answer');
    console.log('PASS: streaming, provider isolation, persistence payload, authenticated thumbnail, modal, incomplete stream');
  } finally {await browser.close();}
})().catch(e=>{console.error(e);process.exitCode=1;});
