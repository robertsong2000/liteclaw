// Opt-in integration test: requires local manual assets/index and model access.
const {chromium} = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
(async()=>{
  const browser = await chromium.launch({headless:true});
  try {
    const page = await browser.newPage({viewport:{width:1280,height:900}});
    const base = process.env.LITECLAW_URL || 'http://127.0.0.1:9999';
    await page.goto(base);
    await page.waitForLoadState('networkidle');
    const authPath = os.homedir()+'/.liteclaw/auth.json';
    const creds = fs.existsSync(authPath) ? JSON.parse(fs.readFileSync(authPath)) : {username:'renault',password:'renault123'};
    const login = await page.request.post(base+'/api/login',{data:creds});
    assert.equal(login.status(),200);
    const {token} = await login.json();
    const headers = {Authorization:'Bearer '+token};
    const id = '6869a21848e1c3f0-figure-1-350';
    assert.equal((await page.request.get(base+'/api/manual-images/'+id)).status(),401);
    const image = await page.request.get(base+'/api/manual-images/'+id,{headers});
    assert.equal(image.status(),200);
    assert.equal(image.headers()['content-type'],'image/png');
    for(const bad of ['unknown','..%2F..%2Fconfig.json']) {
      assert.equal((await page.request.get(base+'/api/manual-images/'+bad,{headers})).status(),404);
    }
    await page.evaluate(token=>{authToken=token;hideLogin();currentSessionId=null;},token);
    await page.selectOption('#model','deepseek-flash');
    await page.check('#no_think');
    for(const [name,auto,question,expectedPage] of [
      ['main-beam',true,'如何手动开启远光灯？请给出拨杆操作图。',145],
      ['auto-beam',true,'如何启用自动远光灯功能？请给出操作图。',[145,146]],
      ['fuses',true,'保险丝盒在哪里？请说明并提供保险丝布局图。',350],
      // PDF pages 43 and 45 both show the actual cable/connector operation.
      ['charging',false,'请先调用 manual-rag 检索：R5 如何开始和结束充电？请结合手册插图回答。',[43,45]],
      ['tyres',true,'胎压标签在哪里？请展示手册图。',334],
      ['warning-lights',true,'Low windscreen washer level warning light — show the manual warning light illustration.',136],
    ]) {
      await page.locator('#auto_rag').setChecked(auto);
      await page.evaluate(async question=>{
        chat.innerHTML=''; messages=[{role:'user',content:question}];
        await streamChat();
      },question);
      const result = await page.evaluate(()=>messages.at(-1));
      console.log(JSON.stringify({auto,content:result.content,images:result.source_images}));
      assert.equal(result.role,'assistant');
      if(!auto) {
        const skillSucceeded = await page.evaluate(()=>messages.some(m => m.role==='tool'
          && m.content?.startsWith('(exit 0)\n[')
          && messages.some(a=>a.tool_calls?.some(tc=>tc.id===m.tool_call_id && tc.function.name==='skill_run'))));
        assert.ok(skillSucceeded,'explicit manual-rag must execute its real entry point successfully, not rely on bash fallback');
      }
      assert.ok(result.source_images?.length>0,'real retrieval must return attachments');
      const featured = result.source_images.filter(i=>i.role==='primary'||i.role==='supporting');
      assert.ok(featured.length>=1 && featured.length<=3,'show only a small question-specific selection');
      assert.equal(result.source_images.filter(i=>i.role==='primary').length,1);
      assert.ok(featured.every(i=>i.kind==='illustration'),'whole pages belong in folded references, not the answer gallery');
      if(name==='main-beam') assert.equal(featured[0].id,'6869a21848e1c3f0-figure-1-145');
      if(name==='auto-beam') assert.ok(featured.every(i=>i.id!=='6869a21848e1c3f0-figure-1-145'),'automatic settings must not display the manual push-arrow figure');
      if(name==='warning-lights') assert.equal(featured[0].id,'6869a21848e1c3f0-symbol-7-136');
      const expectedPages = Array.isArray(expectedPage) ? expectedPage : [expectedPage];
      assert.ok(featured.some(i=>expectedPages.includes(i.page)), name+' must feature the corresponding operation');
      assert.ok(await page.locator('.manual-primary').evaluate(el=>el===el.parentElement.firstElementChild));
      assert.equal(await page.locator('.manual-references[open]').count(),0);
      await page.waitForFunction(()=>{
        const imgs=[...document.querySelectorAll('.manual-gallery img')];
        return imgs.length>0 && imgs.every(i=>i.naturalWidth>0);
      });
      await page.locator('.manual-gallery').scrollIntoViewIfNeeded();
      await page.screenshot({path:'/tmp/manual-images-'+name+'.png',fullPage:true});
      await page.locator('.manual-gallery button').first().click();
      await page.screenshot({path:'/tmp/manual-images-enlarged.png'});
      await page.keyboard.press('Escape');
      // Restore through the production history renderer, not duplicated DOM logic.
      await page.evaluate(()=>renderChatFromMessages());
      assert.ok(await page.locator('.manual-gallery').count()>0);
    }
    await page.setViewportSize({width:390,height:844});
    await page.locator('.manual-gallery').scrollIntoViewIfNeeded();
    await page.screenshot({path:'/tmp/manual-images-mobile.png',fullPage:true});
    assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth),'mobile must not overflow');
    const restored=await page.evaluate(()=>messages);
    const sessions = await (await page.request.get(base+'/api/history',{headers})).json();
    if(sessions.sessions.length<49) {
      const sessionId='manual-images-test-'+Date.now();
      try {
        const save=await page.request.post(base+'/api/history',{headers,data:{id:sessionId,title:'Image persistence test',messages:restored,updated:Date.now()}});
        assert.equal(save.status(),200);
        const read=await (await page.request.get(base+'/api/history/'+sessionId,{headers})).json();
        assert.deepEqual(read.messages.at(-1).source_images,restored.at(-1).source_images);
      } finally {await page.request.delete(base+'/api/history/'+sessionId,{headers});}
    } else {throw Error('History persistence test skipped to preserve full user history');}
    console.log('PASS: live auto/tool RAG, auth, invalid paths, persistence, modal, mobile');
  } finally {await browser.close();}
})().catch(e=>{console.error(e);process.exitCode=1;});
