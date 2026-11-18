"""Open the web app in Playwright WebKit (Safari's engine) with an iPhone profile and report what happens.
usage: webkit_test.py <url> [screenshot.png]"""
import sys
from playwright.sync_api import sync_playwright
url = sys.argv[1]; shot = sys.argv[2] if len(sys.argv) > 2 else None
with sync_playwright() as pw:
    b = pw.webkit.launch(headless=True)
    ctx = b.new_context(**pw.devices['iPhone 14'])
    page = ctx.new_page(); logs = []; bad = []
    page.on('console', lambda m: logs.append(f'{m.type}: {m.text}'))
    page.on('pageerror', lambda e: logs.append(f'pageerror: {e}'))
    page.on('response', lambda r: bad.append(f'{r.status} {r.url}') if r.status >= 400 else None)
    page.goto(url, wait_until='load')
    try:
        page.wait_for_selector('#btnStart:not([disabled])', timeout=25000)
        btn = page.inner_text('#btnStart'); page.tap('#btnStart'); page.wait_for_timeout(3000)
    except Exception as e:
        btn = f'never enabled ({type(e).__name__}); text: ' + (page.inner_text('#btnStart') if page.query_selector('#btnStart') else '?')
    info = page.evaluate("""() => { const S = window.__app || {}; const e = S.engine; return {
        ctxState: S.ctx && S.ctx.state, rate: S.ctx && S.ctx.sampleRate, mock: S.mock, started: S.started,
        haveFrame: !!(e && e.frame), rpm: e && e.frame ? e.frame[2] : null, quality: e && e.stats && e.stats.quality, load: e && e.stats && e.stats.load,
        ua: navigator.userAgent.slice(0, 60) }; }""")
    if shot: page.screenshot(path=shot)
    b.close()
print('start button:', btn); print('state:', info); print('failed requests:', bad or 'none'); print('console:'); [print('  ', l) for l in logs] if logs else print('   clean')
