package dev.r1os.host

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Bitmap
import android.graphics.Canvas
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.util.Base64
import android.view.KeyCharacterMap
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.ViewGroup
import android.webkit.PermissionRequest
import android.webkit.WebChromeClient
import android.webkit.WebResourceRequest
import android.webkit.WebResourceResponse
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.FrameLayout
import dev.r1os.host.net.Ev
import dev.r1os.host.net.R1d
import org.json.JSONObject
import org.json.JSONTokener
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream

/**
 * The R1's browser (ADR-0054): a WebView the R1's agents drive through r1d (`browser {id, op, agent, chat, args}`;
 * crate::browser), answering each request at `POST /api/browser/result`. Its tools are like Claude in Chrome's: open,
 * read (the page as lines with element refs), text, find, click, type, key, select, scroll, screenshot, back, close,
 * and ask_user (you take over: "log me in to this site", then done).
 *
 * - **Where it lives:** behind everything in the shell while nobody's watching (so pages still lay out and draw), and in
 *   [dev.r1os.host.shell.BrowserScreen] when you are. Made on first use; gone two minutes after the last.
 * - **Where it goes:** web pages only. Every request a page makes to the R1 itself or the local network is refused, and
 *   so is anything but http and https (no files, intents or app links). No camera, microphone or location for pages,
 *   no downloads.
 * - **Sign-ins** are the WebView's own cookies. Password fields refuse the agent: you type those after ask_user.
 */
@SuppressLint("StaticFieldLeak")
object AgentBrowser {
    private val main = Handler(Looper.getMainLooper())
    private var web: WebView? = null
    private var back: FrameLayout? = null
    /** Whose it is now: the agent and chat of the last request. */
    var agent: String? = null
        private set
    var chat: String? = null
        private set
    /** You opened it yourself ("go to amazon.com"), and no agent has used it since. */
    var mine = false
        private set
    /** A link that was a file (a PDF), which the browser can't show: the header says so until the next page. */
    var note: String? = null
        private set
    /** The page's address, for the screen's header. */
    var url: String = ""
        private set
    /** You're asked to do something (ask_user): its words, and the request to answer when you're done. */
    var asking: String? = null
        private set
    private var askId: String? = null
    private var lastUse = 0L
    private var loadDone: ((Boolean) -> Unit)? = null
    private var navigating = false
    private val listeners = ArrayList<() -> Unit>()

    /** The screen's hook: what it shows changed (a page loaded, an ask came or went). */
    fun onChange(f: () -> Unit): () -> Unit {
        listeners.add(f)
        return { listeners.remove(f) }
    }

    private fun changed() = listeners.toList().forEach { it() }

    /** Called by the shell once: where the WebView waits while nobody's watching (behind everything). */
    fun backContainer(context: Context): FrameLayout = FrameLayout(context).also { back = it }

    /** The WebView, made on first use. */
    fun view(context: Context): WebView = web ?: make(context).also { web = it; park() }

    /** Back behind everything (the screen let it go). */
    fun park() {
        val w = web ?: return
        val b = back ?: return
        if (w.parent === b) return
        (w.parent as? ViewGroup)?.removeView(w)
        w.scaleX = 1f
        w.scaleY = 1f
        b.addView(w, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
    }

    val open: Boolean get() = web != null

    @SuppressLint("SetJavaScriptEnabled")
    private fun make(context: Context): WebView = WebView(context).apply {
        settings.javaScriptEnabled = true
        settings.domStorageEnabled = true
        settings.allowFileAccess = false
        settings.allowContentAccess = false
        settings.setGeolocationEnabled(false)
        settings.mediaPlaybackRequiresUserGesture = true
        settings.javaScriptCanOpenWindowsAutomatically = false
        settings.setSupportMultipleWindows(false)
        settings.builtInZoomControls = false
        android.webkit.CookieManager.getInstance().setAcceptCookie(true)
        setBackgroundColor(android.graphics.Color.WHITE)
        webViewClient = object : WebViewClient() {
            override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean = !allowed(request.url)

            override fun shouldInterceptRequest(view: WebView, request: WebResourceRequest): WebResourceResponse? =
                if (allowed(request.url)) null else WebResourceResponse("text/plain", "utf-8", 403, "Forbidden", emptyMap(), ByteArrayInputStream(ByteArray(0)))

            override fun onPageStarted(view: WebView, u: String?, favicon: Bitmap?) {
                navigating = true
                note = null
                this@AgentBrowser.url = u.orEmpty()
                changed()
            }

            override fun onPageFinished(view: WebView, u: String?) {
                navigating = false
                this@AgentBrowser.url = u.orEmpty()
                changed()
                loadDone?.let { loadDone = null; it(true) }
            }
        }
        webChromeClient = object : WebChromeClient() {
            override fun onPermissionRequest(request: PermissionRequest) = request.deny()
        }
        // No downloads: a file link (a PDF) is said, not saved.
        setDownloadListener { _, _, _, mime, _ ->
            val kind = when {
                mime.contains("pdf") -> "a pdf"
                mime.startsWith("image/") -> "an image file"
                else -> "a file"
            }
            note = "$kind: the r1's browser can't show it"
            loadDone?.let { loadDone = null; it(false) }
            changed()
        }
    }

    /** Web pages only: http(s), and never the R1 itself or the local network (crate::browser::private_host). */
    fun allowed(u: Uri): Boolean {
        val scheme = u.scheme?.lowercase() ?: return false
        if (scheme == "about" || scheme == "data" || scheme == "blob") return true
        if (scheme != "http" && scheme != "https") return false
        val h = u.host?.lowercase()?.trim('[', ']') ?: return false
        if (h == "localhost" || h.endsWith(".localhost") || h.endsWith(".local") || h.endsWith(".lan") || h.endsWith(".internal")) return false
        if (!h.contains('.') && !h.contains(':')) return false
        val v4 = h.split('.').mapNotNull { it.toIntOrNull() }
        if (v4.size == 4 && h.all { it.isDigit() || it == '.' }) {
            val (a, b) = v4[0] to v4[1]
            return !(a == 127 || a == 10 || a == 0 || (a == 172 && b in 16..31) || (a == 192 && b == 168) || (a == 169 && b == 254) || (a == 100 && b in 64..127))
        }
        if (h.contains(':')) return !(h == "::1" || h.startsWith("fc") || h.startsWith("fd") || h.startsWith("fe80"))
        return true
    }

    // ---- requests from r1d ------------------------------------------------------------------------------

    /** `browser {id, op, agent, chat, args}`: does it, and answers r1d. */
    fun handle(context: Context, ev: Ev) {
        val id = ev.str("id") ?: return
        val op = ev.str("op").orEmpty()
        // Its chat's turn was stopped (T-013): an ask of it goes, and the browser stays whoever's it was.
        if (op == "stopped") {
            if (askId != null && ev.str("chat") == chat) answerAsk(false)
            return
        }
        val args = ev.json.optJSONObject("args") ?: JSONObject()
        // A request from System One (what you said) names no agent: the browser stays the agent's that was using it.
        ev.str("agent")?.takeIf { it.isNotEmpty() }?.let { agent = it; mine = false }
        ev.str("chat")?.takeIf { it.isNotEmpty() }?.let { chat = it }
        lastUse = SystemClock.uptimeMillis()
        main.removeCallbacks(idle)
        main.postDelayed(idle, IDLE_MS)
        if (op == "close") {
            close()
            reply(id, "Closed the browser.")
            return
        }
        val w = view(context)
        when (op) {
            "open" -> load(w, args.optString("url")) { ok -> js(w, "__r1b.text()") { r -> reply(id, summary(r, if (ok) null else "it took a while to load")) } }
            "read" -> js(w, "__r1b.read(${q(args.optString("filter", "interactive"))})") { reply(id, it) }
            "text" -> js(w, "__r1b.text()") { reply(id, it) }
            "find" -> js(w, "__r1b.find(${q(args.optString("query"))})") { reply(id, it) }
            "describe" -> js(w, "__r1b.describe(${q(args.optString("ref"))},${args.optDouble("x", -1.0)},${args.optDouble("y", -1.0)})") { reply(id, it) }
            "click" -> js(w, "__r1b.where(${q(args.optString("ref"))},${args.optDouble("x", -1.0)},${args.optDouble("y", -1.0)})") { r ->
                // A real tap where the element is (pages trust it, unlike a script's click), then what came of it.
                val p = runCatching { JSONObject(r) }.getOrNull()
                if (p == null || p.has("error")) { fail(id, p?.optString("error") ?: "couldn't find it on the page"); return@js }
                val x = p.optDouble("x").toFloat()
                val y = p.optDouble("y").toFloat()
                // The agent's pointer goes there first, if you're watching (T-137); the tap as it lands.
                AgentPointer.browser.aim(x, y) {
                    tap(w, x, y)
                    settle(w) { reply(id, JSONObject().put("url", url).put("text", "Clicked ${p.optString("what")}.").toString()) }
                }
            }
            "type" -> pointAt(w, args.optString("ref")) { typeInto(id, w, args.optString("ref"), args.optString("text"), args.optBoolean("submit")) }
            "select" -> pointAt(w, args.optString("ref")) { js(w, "__r1b.select(${q(args.optString("ref"))},${q(args.optString("value"))})") { reply(id, it) } }
            "key" -> {
                AgentPointer.browser.pulse()
                key(w, args.optString("key"))
                settle(w) { reply(id, JSONObject().put("url", url).put("text", "Pressed ${args.optString("key")}.").toString()) }
            }
            "scroll" -> {
                val go = { js(w, "__r1b.scroll(${q(args.optString("direction", "down"))},${args.optInt("amount", 1)})") { reply(id, it) } }
                // A short stroke in the middle of the page, the way a finger would scroll it.
                val d = w.height * 0.15f * (if (args.optString("direction", "down") == "up") -1 else 1)
                if (AgentPointer.browser.watched && w.height > 0) AgentPointer.browser.drag(w.width / 2f, w.height / 2f + d, w.width / 2f, w.height / 2f - d, 220) { go() }
                else go()
            }
            "back" -> if (w.canGoBack()) { load(w, null) { js(w, "__r1b.text()") { r -> reply(id, summary(r, null)) } }; w.goBack() } else fail(id, "Nothing to go back to.")
            "screenshot" -> main.postDelayed({ screenshot(id, w) }, 150)
            "reload" -> { load(w, null) { reply(id, JSONObject().put("url", url).put("text", "Reloaded.").toString()) }; w.reload() }
            "ask_user" -> {
                asking = args.optString("why").ifEmpty { "Take over the browser" }
                askId = id
                changed()
            }
            else -> fail(id, "Unknown browser op `$op`.")
        }
    }

    /** The wheel on the screen: the page scrolls (a tick is a little). */
    fun scrollBy(ticks: Int) {
        lastUse = SystemClock.uptimeMillis()
        web?.evaluateJavascript("scrollBy(0, ${'$'}{ticks * 90})", null)
    }

    /** You open it ("go to amazon.com", or "the browser"): on that page if there's one. */
    fun openForYou(context: Context, u: String?) {
        val w = view(context)
        mine = true
        chat = null
        lastUse = SystemClock.uptimeMillis()
        main.removeCallbacks(idle)
        main.postDelayed(idle, IDLE_MS)
        if (u != null && allowed(Uri.parse(u))) w.loadUrl(u)
        changed()
    }

    /** You're done with what an agent asked (or not): its tool goes on. */
    fun answerAsk(done: Boolean) {
        val id = askId ?: return
        askId = null
        asking = null
        changed()
        if (done) reply(id, JSONObject().put("url", url).put("text", "The user says they're done. Read the page again to see where things are.").toString())
        else fail(id, "The user didn't want to do that now.")
    }

    // ---- doing things ------------------------------------------------------------------------------------

    private fun load(w: WebView, u: String?, done: (Boolean) -> Unit) {
        var finished = false
        val timeout = Runnable { if (!finished) { finished = true; loadDone = null; done(false) } }
        loadDone = { ok -> if (!finished) { finished = true; main.removeCallbacks(timeout); main.postDelayed({ done(ok) }, 400) } }
        main.postDelayed(timeout, LOAD_MS)
        if (u != null) w.loadUrl(u)
    }

    /** After an act: a moment for the page to answer, longer if it started loading another. */
    private fun settle(w: WebView, done: () -> Unit) {
        main.postDelayed({
            if (navigating) load(w, null) { done() } else done()
        }, 900)
    }

    /** Runs the helper (put in first: a page that loaded has lost it), then `expr`; its string result. */
    private fun js(w: WebView, expr: String, done: (String) -> Unit) {
        w.evaluateJavascript("$HELPER;($expr)") { raw ->
            val s = runCatching { JSONTokener(raw).nextValue() as? String }.getOrNull()
            done(s ?: JSONObject().put("error", "the page didn't answer (still loading?)").toString())
        }
    }

    /**
     * The agent's pointer to a field before typing into or choosing in it, when you're watching (T-137): it asks the page
     * where the field is first (which scrolls it into view); with nobody watching, or no ref, straight on.
     */
    private fun pointAt(w: WebView, ref: String, then: () -> Unit) {
        if (ref.isEmpty() || !AgentPointer.browser.watched) { then(); return }
        js(w, "__r1b.where(${q(ref)},-1,-1)") { r ->
            val p = runCatching { JSONObject(r) }.getOrNull()
            if (p == null || p.has("error")) then() else AgentPointer.browser.aim(p.optDouble("x").toFloat(), p.optDouble("y").toFloat(), press = false, act = then)
        }
    }

    /**
     * Types as a person does: the field focused and its words selected (so they're replaced), then real keys, and a real
     * Enter to send. Autocompletes and pages that listen for keys hear them; weather.gov's search box didn't hear words
     * set from a script (2026-10-12). Words the keyboard map can't make (emoji) are set from the script as before.
     */
    private fun typeInto(id: String, w: WebView, ref: String, text: String, submit: Boolean) {
        val events = KeyCharacterMap.load(KeyCharacterMap.VIRTUAL_KEYBOARD).getEvents(text.toCharArray())
        if (events == null) {
            js(w, "__r1b.type(${q(ref)},${q(text)},$submit)") { r -> if (submit) settle(w) { reply(id, r) } else reply(id, r) }
            return
        }
        val bySetting = { js(w, "__r1b.type(${q(ref)},${q(text)},$submit)") { r -> if (submit) settle(w) { reply(id, r) } else reply(id, r) } }
        js(w, "__r1b.focus(${q(ref)})") { r ->
            val p = runCatching { JSONObject(r) }.getOrNull()
            if (p == null || p.has("error")) { reply(id, r); return@js }
            // Dates, times, colours and ranges don't take typed keys in Chrome (httpbin's delivery time stayed empty,
            // 2026-10-13): their value is set.
            if (p.optString("kind") in SET_ONLY) { bySetting(); return@js }
            events.forEach { w.dispatchKeyEvent(it) }
            val what = p.optString("what")
            main.postDelayed({
                js(w, "__r1b.value()") { v ->
                    val now = runCatching { JSONObject(v).optString("value") }.getOrDefault("")
                    when {
                        // The keys didn't go in (a field that ignores them): set as before.
                        !now.contains(text, ignoreCase = true) -> bySetting()
                        submit -> {
                            key(w, "enter")
                            settle(w) { reply(id, JSONObject().put("url", url).put("text", "Typed into $what and pressed Enter.").toString()) }
                        }
                        else -> reply(id, JSONObject().put("url", url).put("text", "Typed into $what: it now says “$now”.").toString())
                    }
                }
            }, KEYS_MS)
        }
    }

    private fun tap(w: WebView, x: Float, y: Float) {
        val t = SystemClock.uptimeMillis()
        val down = MotionEvent.obtain(t, t, MotionEvent.ACTION_DOWN, x, y, 0)
        val up = MotionEvent.obtain(t, t + 60, MotionEvent.ACTION_UP, x, y, 0)
        w.dispatchTouchEvent(down)
        w.dispatchTouchEvent(up)
        down.recycle()
        up.recycle()
    }

    private fun key(w: WebView, k: String) {
        val code = when (k.lowercase()) {
            "enter", "return" -> KeyEvent.KEYCODE_ENTER
            "tab" -> KeyEvent.KEYCODE_TAB
            "escape", "esc" -> KeyEvent.KEYCODE_ESCAPE
            "backspace" -> KeyEvent.KEYCODE_DEL
            "arrowdown", "down" -> KeyEvent.KEYCODE_DPAD_DOWN
            "arrowup", "up" -> KeyEvent.KEYCODE_DPAD_UP
            "arrowleft", "left" -> KeyEvent.KEYCODE_DPAD_LEFT
            "arrowright", "right" -> KeyEvent.KEYCODE_DPAD_RIGHT
            "space" -> KeyEvent.KEYCODE_SPACE
            "pagedown" -> KeyEvent.KEYCODE_PAGE_DOWN
            "pageup" -> KeyEvent.KEYCODE_PAGE_UP
            else -> KeyEvent.KEYCODE_UNKNOWN
        }
        if (code == KeyEvent.KEYCODE_UNKNOWN) return
        w.dispatchKeyEvent(KeyEvent(KeyEvent.ACTION_DOWN, code))
        w.dispatchKeyEvent(KeyEvent(KeyEvent.ACTION_UP, code))
    }

    private fun screenshot(id: String, w: WebView) {
        if (w.width == 0 || w.height == 0) { fail(id, "Nothing's open yet."); return }
        val scale = minOf(1f, SHOT_W / w.width.toFloat())
        val bmp = Bitmap.createBitmap((w.width * scale).toInt(), (w.height * scale).toInt(), Bitmap.Config.ARGB_8888)
        val c = Canvas(bmp)
        c.scale(scale, scale)
        w.draw(c)
        val out = ByteArrayOutputStream()
        bmp.compress(Bitmap.CompressFormat.JPEG, 70, out)
        bmp.recycle()
        reply(id, JSONObject().put("image", Base64.encodeToString(out.toByteArray(), Base64.NO_WRAP)).put("url", url)
            .put("width", (w.width * scale).toInt()).put("height", (w.height * scale).toInt()))
    }

    private val idle = Runnable { if (askId == null && SystemClock.uptimeMillis() - lastUse >= IDLE_MS && !watching) close() }

    /** The screen's showing it: it stays while you look. */
    var watching = false

    fun close() {
        main.removeCallbacks(idle)
        askId?.let { answerAsk(false) }
        web?.let { w ->
            (w.parent as? ViewGroup)?.removeView(w)
            w.stopLoading()
            w.destroy()
        }
        web = null
        url = ""
        changed()
    }

    // ---- answering r1d -----------------------------------------------------------------------------------

    private fun reply(id: String, result: Any) = R1d.post("/api/browser/result", JSONObject().put("id", id).put("ok", true).put("result", parsed(result)), onError = {}) {}

    private fun fail(id: String, why: String) = R1d.post("/api/browser/result", JSONObject().put("id", id).put("ok", false).put("error", why), onError = {}) {}

    /** The helper's JSON (an object) as itself, so r1d gets its fields; an error it reports becomes a failure's words. */
    private fun parsed(r: Any): Any = when (r) {
        is String -> runCatching { JSONObject(r) }.getOrNull()?.let { o -> if (o.has("error")) o.optString("error") else o } ?: r
        else -> r
    }

    /** A page's first words for an agent that just opened it. */
    private fun summary(r: String, note: String?): String {
        val o = runCatching { JSONObject(r) }.getOrNull() ?: return r
        val text = o.optString("text").take(OPEN_CHARS)
        return JSONObject().put("url", o.optString("url")).put("text", listOfNotNull(o.optString("title").ifEmpty { null }, note, text).joinToString("\n")).toString()
    }

    private fun q(s: String) = JSONObject.quote(s)

    private const val IDLE_MS = 2 * 60_000L
    /** After typing keys, before reading the field or sending it: the page takes them in. */
    private const val KEYS_MS = 150L
    /** Fields whose value is set rather than typed. */
    private val SET_ONLY = setOf("time", "date", "datetime-local", "month", "week", "color", "range")
    private const val LOAD_MS = 20_000L
    private const val SHOT_W = 480f
    private const val OPEN_CHARS = 3000

    /** In the page: the element refs and what the tools do (idempotent: put in before every request). */
    private const val HELPER = """(function(){
if (window.__r1b) return;
var n = 0;
function vis(el){ var r=el.getBoundingClientRect(); if(r.width<=0||r.height<=0) return false; var s=getComputedStyle(el); return s.visibility!=='hidden'&&s.display!=='none'&&s.opacity!=='0'; }
function role(el){ var t=el.tagName.toLowerCase(), r=el.getAttribute('role'); if(r) return r; if(t==='a') return 'link'; if(t==='button'||t==='summary') return 'button';
 if(t==='input'){ var ty=(el.type||'text').toLowerCase(); if(ty==='button'||ty==='submit'||ty==='reset') return 'button'; if(ty==='checkbox'||ty==='radio') return ty; if(ty==='password') return 'password';
  if(ty==='search'||/^(q|query|search|s|keywords?|term)$/i.test(el.name||'')||/search/i.test((el.id||'')+' '+(el.className||''))) return 'searchbox'; return 'textbox'; }
 if(t==='textarea') return 'textbox'; if(t==='select') return 'combobox'; if(/^h[1-6]$/.test(t)) return 'heading'; if(t==='img') return 'img'; return t; }
function lab(el){ try { if(el.labels&&el.labels.length) return el.labels[0].innerText; } catch(e){} return ''; }
function name(el){ return (el.getAttribute('aria-label')||lab(el)||el.getAttribute('alt')||el.getAttribute('title')||el.getAttribute('placeholder')||el.innerText||(el.type!=='password'?el.value:'')||(role(el)==='searchbox'?'search':'')||el.getAttribute('name')||el.id||'').replace(/\s+/g,' ').trim().slice(0,80); }
function inter(el){ var t=el.tagName.toLowerCase(); return ['a','button','input','textarea','select','summary'].indexOf(t)>=0||el.hasAttribute('onclick')||el.isContentEditable||['button','link','checkbox','radio','tab','menuitem','option','switch','textbox','combobox','searchbox'].indexOf(el.getAttribute('role')||'')>=0; }
function ref(el){ var r=el.getAttribute('data-r1ref'); if(!r){ r='r'+(++n); el.setAttribute('data-r1ref',r); } return r; }
function get(r,x,y){ return r ? document.querySelector('[data-r1ref="'+r+'"]') : (x>=0 ? document.elementFromPoint(x,y) : null); }
function out(o){ o.url=location.href; return JSON.stringify(o); }
window.__r1b = {
 read: function(f){ var all=f==='all', lines=[], els=document.querySelectorAll(all?'a,button,input,textarea,select,summary,[role],[onclick],[contenteditable],h1,h2,h3,h4,img,label,p,li,td':'a,button,input,textarea,select,summary,[role],[onclick],[contenteditable]');
  for(var i=0;i<els.length&&lines.length<250;i++){ var el=els[i]; if(!vis(el)) continue; var it=inter(el); if(!all&&!it) continue; var nm=name(el); if(!nm&&!it) continue;
   var l=role(el)+(nm?' "'+nm+'"':''); if((el.tagName==='INPUT'&&el.type!=='password'||el.tagName==='TEXTAREA')&&el.value) l+=' value="'+String(el.value).slice(0,60)+'"'; if(el.checked) l+=' (checked)'; if(el.disabled) l+=' (disabled)'; if(it) l+=' ['+ref(el)+']'; lines.push(l); }
  return out({title:document.title, text:lines.join('\n')||'(nothing to act on: try browser_text or a screenshot)'}); },
 text: function(){ var m=document.querySelector('main,article,[role=main]')||document.body; return out({title:document.title, text:((m&&m.innerText)||'').replace(/\n{3,}/g,'\n\n').slice(0,20000)}); },
 find: function(q){ q=String(q).toLowerCase(); var res=[], els=document.querySelectorAll('body *'); for(var i=0;i<els.length&&res.length<20;i++){ var el=els[i]; if(el.children.length>4||!vis(el)) continue; var nm=name(el); if(nm.toLowerCase().indexOf(q)<0) continue; res.push(role(el)+' "'+nm+'" ['+ref(el)+']'); } return out({text:res.length?res.join('\n'):'nothing matches "'+q+'"'}); },
 describe: function(r,x,y){ var el=get(r,x,y); if(!el) return out({}); return out({name:name(el), role:role(el), site:location.hostname, password:el.type==='password'}); },
 where: function(r,x,y){ var el=get(r,x,y); if(!el) return out({error:'no element '+(r||(x+','+y))+' on the page: read it again'}); el.scrollIntoView({block:'center',inline:'center'}); var rc=el.getBoundingClientRect(), d=window.devicePixelRatio||1;
  return out({x:(rc.left+rc.width/2)*d, y:(rc.top+rc.height/2)*d, what:role(el)+' "'+name(el)+'"'}); },
 focus: function(r){ var el=r?get(r):document.activeElement; if(!el||el===document.body) return out({error:'no field: give its ref (browser_read)'}); if(el.type==='password') return out({error:'that is a password field: ask the user to type it themselves (browser_ask_user)'});
  el.scrollIntoView({block:'center'}); el.focus(); try{ if(el.select) el.select(); else if(el.isContentEditable){ var s=getSelection(), g=document.createRange(); g.selectNodeContents(el); s.removeAllRanges(); s.addRange(g); } }catch(e){} return out({what:role(el)+' "'+name(el)+'"', kind:String(el.type||el.tagName||'').toLowerCase()}); },
 value: function(){ var el=document.activeElement; return out({value: el ? String(el.value!==undefined ? el.value : (el.innerText||'')).slice(0,200) : ''}); },
 type: function(r,text,submit){ var el=r?get(r):document.activeElement; if(!el||el===document.body) return out({error:'no field: give its ref (browser_read)'}); if(el.type==='password') return out({error:'that is a password field: ask the user to type it themselves (browser_ask_user)'});
  el.scrollIntoView({block:'center'}); el.focus(); if(el.isContentEditable){ document.execCommand('insertText',false,text); } else { var p=el.tagName==='TEXTAREA'?HTMLTextAreaElement.prototype:HTMLInputElement.prototype; var set=Object.getOwnPropertyDescriptor(p,'value').set; set.call(el,/^(time|date|datetime-local|month|week|color|range)$/.test(el.type)?text:(el.value||'')+text); el.dispatchEvent(new Event('input',{bubbles:true})); el.dispatchEvent(new Event('change',{bubbles:true})); }
  if(submit){ el.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',code:'Enter',keyCode:13,which:13,bubbles:true})); if(el.form){ if(el.form.requestSubmit) el.form.requestSubmit(); else el.form.submit(); } }
  var now=el.isContentEditable?el.innerText:el.value; return out({text:'Typed into '+role(el)+' "'+name(el)+'"'+(submit?' and sent it.':': it now says \u201c'+String(now).slice(0,80)+'\u201d.')}); },
 select: function(r,v){ var el=get(r); if(!el||el.tagName!=='SELECT') return out({error:'that is not a list to choose from'}); var o=null; for(var i=0;i<el.options.length;i++){ var x=el.options[i]; if(x.value===v||x.text.trim().toLowerCase()===String(v).toLowerCase()) o=x; }
  if(!o){ var names=[]; for(var j=0;j<el.options.length;j++) names.push(el.options[j].text.trim()); return out({error:'no option "'+v+'": '+names.join(', ')}); } el.value=o.value; el.dispatchEvent(new Event('change',{bubbles:true})); return out({text:'Chose '+o.text.trim()+'.'}); },
 scroll: function(dir,a){ if(dir==='top'){ scrollTo(0,0); } else if(dir==='bottom'){ scrollTo(0,document.documentElement.scrollHeight); } else { var h=Math.round(innerHeight*0.8)*(a||1); scrollBy(0,dir==='up'?-h:h); } return out({text:'Scrolled '+dir+': '+Math.round(scrollY)+' of '+Math.max(0,document.documentElement.scrollHeight-innerHeight)+' px.'}); }
};
})()"""
}
