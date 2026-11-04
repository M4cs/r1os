package dev.r1os.host.shell

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Color
import android.view.Gravity
import android.view.View
import android.view.ViewGroup
import android.widget.FrameLayout
import android.widget.ImageView
import android.widget.LinearLayout
import android.widget.TextView
import dev.r1os.host.net.Ev
import dev.r1os.host.net.R1d
import dev.r1os.host.ui.CharacterView
import dev.r1os.host.ui.Wardrobe
import dev.r1os.host.ui.Icons
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dp
import dev.r1os.host.ui.dpi
import dev.r1os.host.ui.text
import org.json.JSONArray
import org.json.JSONObject

// Setup steps shared by onboarding and Settings (docs/specs/experience.md §2.4; ported from
// shell/src/setup.tsx): Wi-Fi, making an agent, and connecting a brain (Claude / Codex).

/** A list panel: a [FocusList] with the usual wheel input. */
abstract class ListPanel(shell: Shell, accent: Int = Theme.accent) : Panel(shell) {
    protected val list = FocusList(context).apply {
        this.accent = accent
        gap(10f)
        setPadding(context.dpi(14f), context.dpi(6f), context.dpi(14f), context.dpi(14f))
    }
    override val view: View get() = list

    /** Input the list doesn't take. */
    open fun extra(ev: Ev): Boolean = false

    override fun onInput(ev: Ev): Boolean = extra(ev) || listInput(list, ev) { onTop() }

    override fun onBackLit(lit: Boolean) = backLit(list, lit)

    /** A step's title: big, bold, lowercase (ADR-0021). */
    protected fun title(s: String) = list.note(s, Theme.FG, 28f, weight = 700)

    /** A small character beside a title (`.setup-row`). */
    protected fun titleRow(a: JSONObject, s: String, size: Float = 72f, state: String = "idle") {
        val row = LinearLayout(context).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            clipChildren = false
            addView(CharacterView(context, context.dpi(size)).apply { set(a.optString("form"), a.optString("color"), a.optString("finish"), state) },
                LinearLayout.LayoutParams(context.dpi(size), context.dpi(size)))
            addView(context.text(24f, 700).apply { text = s; setPadding(context.dpi(12f), 0, 0, 0) },
                LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
        }
        list.place(row)
    }
}

// ---- Wi-Fi ----------------------------------------------------------------------------------------

/** Join a network: the list r1d scans, a password, then "You're online". */
class WifiPanel(shell: Shell, private val doneLabel: String = "Continue", private val onDone: () -> Unit) : ListPanel(shell) {
    private var status: JSONObject? = null
    private var nets: List<JSONObject> = emptyList()
    private var picked: JSONObject? = null
    private var busy = false
    private var err: String? = null
    private var choosing = false
    private var shown = false
    private var drawn = ""
    private var drawnMode = ""
    private val poll = object : Runnable {
        override fun run() {
            load()
            R1d.main.postDelayed(this, 4000)
        }
    }

    init { render() }

    private fun load(): Unit = R1d.get("/api/wifi", onError = { if (status == null) { err = it; render() } }) { r ->
        status = r.optJSONObject("status")
        val a = r.optJSONArray("networks") ?: JSONArray()
        nets = (0 until a.length()).mapNotNull { a.optJSONObject(it) }
        render()
    }

    override fun onShow() {
        shown = true
        R1d.post("/api/wifi/scan")
        R1d.main.removeCallbacks(poll)
        poll.run()
    }

    override fun onHide() {
        shown = false
        R1d.main.removeCallbacks(poll)
    }

    override fun onDestroy() = onHide()

    override fun onBack(): Boolean {
        if (picked != null) { picked = null; err = null; render(); return true }
        if (choosing && status?.optString("ssid").orEmpty().isNotEmpty()) { choosing = false; render(); return true }
        return false
    }

    private fun ssid() = status?.optString("ssid").orEmpty()

    private fun connect(n: JSONObject, password: String? = null) {
        busy = true
        err = null
        render()
        val body = JSONObject().put("ssid", n.optString("ssid")).put("security", n.optString("security"))
        if (password != null) body.put("password", password)
        R1d.post("/api/wifi/connect", body, onError = { busy = false; err = it; render() }) {
            // Joining takes a few seconds: watch for the network to be the one we're on.
            var tries = 0
            val check = object : Runnable {
                override fun run() {
                    R1d.get("/api/wifi") { r ->
                        val st = r.optJSONObject("status")
                        if (st?.optString("ssid") == n.optString("ssid")) {
                            status = st
                            picked = null
                            choosing = false
                            busy = false
                            render()
                        } else if (++tries >= 25) {
                            busy = false
                            err = "Couldn't join. Check the password and try again."
                            render()
                        } else {
                            R1d.main.postDelayed(this, 1000)
                        }
                    }
                }
            }
            R1d.main.postDelayed(check, 1000)
        }
    }

    private fun render() {
        val p = picked
        // What's on screen; a poll that changes nothing visible doesn't rebuild (it would also take
        // the password field away mid-typing).
        val key = listOf(ssid(), choosing, p?.optString("ssid"), busy, err, if (p == null) nets.take(12).joinToString { it.optString("ssid") + it.optInt("bars") } else "").toString()
        if (key == drawn) return
        drawn = key
        val mode = when { err != null && status == null -> "error"; ssid().isNotEmpty() && !choosing -> "online"; p != null -> "password"; else -> "list" }
        if (mode != drawnMode) { list.clear(); drawnMode = mode }
        list.rebuild {
            when {
                err != null && status == null -> {
                    title("Wi-Fi")
                    note(err!!)
                    button(doneLabel) { onDone() }
                }
                ssid().isNotEmpty() && !choosing -> {
                    title("You're online")
                    place(context.card(LinearLayout(context).apply {
                        orientation = LinearLayout.HORIZONTAL
                        gravity = Gravity.CENTER_VERTICAL
                        addView(ImageView(context).apply { setImageDrawable(Icons.drawable("wifi-high", Theme.OK)) }, LinearLayout.LayoutParams(context.dpi(20f), context.dpi(20f)))
                        addView(context.text(20f, 650).apply { text = ssid(); setPadding(context.dpi(10f), 0, 0, 0) })
                    }))
                    button(doneLabel) { onDone() }
                    button("Use another network", primary = false) { choosing = true; render() }
                }
                p != null -> {
                    title(p.optString("ssid"))
                    val pw = field("Password", password = true, maxLength = 63)
                    button(if (busy) "Joining…" else "Join", enabled = !busy) {
                        val v = pw.text.toString()
                        if (v.length < 8) { err = "Wi-Fi passwords are at least 8 characters."; drawn = ""; render() } else connect(p, v)
                    }
                    button("Back", primary = false) { picked = null; err = null; render() }
                    err?.let { note(it, Theme.BAD, 13f) }
                }
                else -> {
                    title("Choose Wi-Fi")
                    if (nets.isEmpty()) note("Looking for networks…")
                    for (n in nets.take(12)) {
                        val bars = n.optInt("bars")
                        val open = n.optString("security") == "open"
                        val icon = when { bars >= 3 -> "wifi-high"; bars == 2 -> "wifi-medium"; bars == 1 -> "wifi-low"; else -> "wifi-none" }
                        val row = LinearLayout(context).apply {
                            orientation = LinearLayout.HORIZONTAL
                            gravity = Gravity.CENTER_VERTICAL
                            setPadding(context.dpi(12f), context.dpi(12f), context.dpi(12f), context.dpi(12f))
                            background = roundRect(Theme.PANEL, context.dp(14f))
                            addView(ImageView(context).apply {
                                setImageDrawable(Icons.drawable(icon, if (bars >= 3) Theme.OK else Theme.FG))
                                alpha = if (bars >= 3) 1f else if (bars == 2) 0.6f else 0.35f
                            }, LinearLayout.LayoutParams(context.dpi(18f), context.dpi(18f)))
                            addView(context.text(18f, 500).apply { text = n.optString("ssid"); maxLines = 1; ellipsize = android.text.TextUtils.TruncateAt.END; setPadding(context.dpi(12f), 0, 0, 0) },
                                LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
                            if (!open) addView(ImageView(context).apply { setImageDrawable(Icons.drawable("lock-simple", Theme.MUTED)) }, LinearLayout.LayoutParams(context.dpi(15f), context.dpi(15f)))
                        }
                        card(row) { if (busy) Unit else if (open) connect(n) else { picked = n; err = null; render() } }
                    }
                    if (busy) note("Joining…", size = 13f)
                    err?.let { note(it, Theme.BAD, 13f) }
                }
            }
        }
    }
}

// ---- Make your agent ----------------------------------------------------------------------------

/**
 * Shape → colour → finish → name → what it's for → personality. The shape, colour and finish steps
 * turn through choices with the wheel, with a live preview; a click moves on. The last pick saves
 * through `onSave` (make a new agent, or update one), then `onDone` gets the saved agent.
 */
class DesignerPanel(
    shell: Shell,
    initial: JSONObject,
    private val doneLabel: String,
    private val onSave: (JSONObject, (JSONObject) -> Unit, (String) -> Unit) -> Unit,
    private val onDone: (JSONObject) -> Unit,
) : ListPanel(shell, Theme.colorOf(initial.optString("color"))) {
    private val a = JSONObject(initial.toString())
    private var step = "form"
    private var saving = false
    private var err: String? = null
    private var drawnStep = ""
    private var typedName = ""
    private var typedRole = if (ROLES.any { it.first == a.optString("role") }) "" else a.optString("role")
    /** A colour of its own (`#rrggbb`), and whether the colour step is on it (T-060). */
    private var own: String? = a.optString("color").takeIf { it.startsWith("#") }
    private var onOwn = own != null

    // The pick steps' own layout: a big preview, the choice, and its dots.
    private val pickTitle = context.text(22f, 750).apply { gravity = Gravity.CENTER }
    private val preview = CharacterView(context, context.dpi(170f))
    private val pickLabel = context.text(22f, 700).apply { gravity = Gravity.CENTER; minWidth = context.dpi(130f) }
    private val dots = LinearLayout(context).apply { orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER }
    private val pick = LinearLayout(context).apply {
        orientation = LinearLayout.VERTICAL
        gravity = Gravity.CENTER_HORIZONTAL
        clipChildren = false
        setPadding(context.dpi(14f), context.dpi(6f), context.dpi(14f), context.dpi(14f))
        addView(pickTitle)
        addView(preview, LinearLayout.LayoutParams(context.dpi(170f), context.dpi(170f)).apply { topMargin = context.dpi(6f) })
        val arrows = LinearLayout(context).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            addView(arrow("caret-left") { turn(-1) })
            addView(pickLabel)
            addView(arrow("caret-right") { turn(1) })
        }
        addView(arrows, LinearLayout.LayoutParams(LinearLayout.LayoutParams.WRAP_CONTENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(8f) })
        addView(dots, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(10f) })
        addView(context.text(19f, 700, Color.parseColor("#1d0d02")).apply {
            text = "Next"
            gravity = Gravity.CENTER
            setPadding(0, context.dpi(13f), 0, context.dpi(13f))
            setOnClickListener { next() }
            tag = "next"
        }, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(14f) })
        addView(context.text(12f, 500, Theme.MUTED).apply { text = "wheel to change · click to choose"; gravity = Gravity.CENTER; setPadding(0, context.dpi(8f), 0, 0) })
    }
    private val frame = FrameLayout(context).apply {
        addView(pick, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
        addView(list, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
    }
    override val view: View get() = frame

    init { render() }

    private fun arrow(icon: String, onClick: () -> Unit) = ImageView(context).apply {
        setImageDrawable(Icons.drawable(icon, Theme.MUTED))
        setPadding(context.dpi(10f), context.dpi(10f), context.dpi(10f), context.dpi(10f))
        layoutParams = LinearLayout.LayoutParams(context.dpi(44f), context.dpi(44f))
        setOnClickListener { onClick() }
    }

    private fun choices(): List<Pair<String, String>> = when (step) {
        "form" -> CharacterView.FORMS.map { it to Wardrobe.catalog.form(it).label }
        // The palette, then one of your own (T-060): picked like the accent, on the wheel.
        "color" -> Theme.COLORS.keys.map { it to it.replaceFirstChar(Char::uppercase) } + (OWN to if (a.optString("color").startsWith("#")) "Your own" else "Your own…")
        "eyes" -> CharacterView.EYES.map { it to Wardrobe.catalog.eyes(it).label }
        in WEAR_STEPS -> listOf("none" to "None") + Wardrobe.catalog.itemsIn(step).map { it.id to it.label }
        else -> CharacterView.FINISHES.map { it to (FINISH_LABEL[it] ?: it) }
    }

    /** What it wears (T-062), as the designer holds it. */
    private fun wear(): List<String> = a.optJSONArray("wear")?.let { w -> (0 until w.length()).map { w.optString(it) } }.orEmpty()

    /** The wardrobe's items for a slot's step. */
    private fun kind(step: String) = Wardrobe.catalog.itemsIn(step).map { it.id }

    /** The step's current choice: a field, or (what it wears) the one of the step's kind, or none. */
    private fun current(): String = when (step) {
        in WEAR_STEPS -> wear().firstOrNull { it in kind(step) } ?: "none"
        "eyes" -> a.optString("eyes", "classic").ifEmpty { "classic" }
        "color" -> if (onOwn || a.optString("color").startsWith("#")) OWN else a.optString("color")
        else -> a.optString(step)
    }

    private fun turn(d: Int) {
        val list = choices().map { it.first }
        val cur = list.indexOf(current()).coerceAtLeast(0)
        val pick = list[(cur + d + list.size) % list.size]
        if (step == "color") {
            // On your own: the colour you made, if you have; else the last palette one stays while you're on it.
            onOwn = pick == OWN
            if (pick == OWN) own?.let { a.put("color", it) } else a.put("color", pick)
            render()
            return
        }
        if (step in WEAR_STEPS) {
            val others = wear().filter { it !in kind(step) }
            a.put("wear", org.json.JSONArray(if (pick == "none") others else others + pick))
        } else {
            a.put(step, pick)
        }
        render()
    }

    private fun next() {
        step = STEPS.getOrNull(STEPS.indexOf(step) + 1) ?: step
        render()
    }

    /** Your own colour (T-060): hue, vibrance and brightness on the wheel, from the one it has; then on to the finish. */
    private fun pickOwn() {
        shell.push(ColorPickScreen(shell, "agent", Theme.colorOf(a.optString("color"))) { hex ->
            own = hex
            a.put("color", hex)
            onOwn = true
            next()
        })
    }

    override fun onInput(ev: Ev): Boolean {
        if (step in PICKS) {
            if (ev.type == "wheel") { turn(if (ev.int("delta") > 0) 1 else -1); return true }
            if (ev.isTap && step == "color" && current() == OWN) { pickOwn(); return true }
            if (ev.isTap) { next(); return true }
            return false
        }
        if (saving) return ev.type == "wheel" || ev.isTap
        return super.onInput(ev)
    }

    override fun onBack(): Boolean {
        val prev = STEPS.getOrNull(STEPS.indexOf(step) - 1)?.takeIf { step != "form" } ?: return false
        if (saving) return true
        step = prev
        render()
        return true
    }

    private fun save() {
        saving = true
        err = null
        render()
        onSave(a, { saved -> onDone(saved) }, { e -> saving = false; err = e; render() })
    }

    private fun render() {
        val picking = step in PICKS
        pick.visibility = if (picking) View.VISIBLE else View.GONE
        list.visibility = if (picking) View.GONE else View.VISIBLE
        val c = Theme.colorOf(a.optString("color"))
        list.accent = c
        if (picking) {
            pickTitle.text = when (step) {
                "form" -> "Pick a shape"
                "color" -> "Pick a color"
                "eyes" -> "Pick its eyes"
                in WEAR_STEPS -> "Something ${Wardrobe.catalog.slot(step)?.label?.lowercase()}"
                else -> "Pick a finish"
            }
            preview.set(a.optString("form"), a.optString("color"), a.optString("finish"), "idle", eyes = a.optString("eyes", "classic"), wear = wear())
            val cs = choices()
            val cur = current()
            pickLabel.text = cs.firstOrNull { it.first == cur }?.second ?: cur
            pick.findViewWithTag<TextView>("next").background = roundRect(c, context.dp(14f))
            dots.removeAllViews()
            for ((id, _) in cs) {
                val on = id == cur
                dots.addView(View(context).apply {
                    background = android.graphics.drawable.GradientDrawable().apply {
                        shape = android.graphics.drawable.GradientDrawable.OVAL
                        setColor(if (step == "color") Theme.colorOf(if (id == OWN) own ?: "#8a8f98" else id) else Theme.SUBTLE)
                        if (on) setStroke(context.dpi(2f), Theme.FG)
                    }
                }, LinearLayout.LayoutParams(context.dpi(if (on) 13f else 10f), context.dpi(if (on) 13f else 10f)).apply { setMargins(context.dpi(3f), 0, context.dpi(3f), 0) })
            }
            return
        }
        val name = a.optString("name")
        val fresh = step != drawnStep
        if (fresh) { list.clear(); drawnStep = step }
        list.rebuild {
            when (step) {
                "name" -> {
                    titleRow(a, "Name it")
                    val names = if (name in NAMES) NAMES else listOf(name) + NAMES
                    val flow = Flow(context)
                    for (n in names.take(8)) {
                        val chip = context.text(17f, 500).apply {
                            text = n
                            setPadding(context.dpi(14f), context.dpi(8f), context.dpi(14f), context.dpi(8f))
                            background = roundRect(Theme.FOCUS, context.dp(999f), if (n == name) context.dpi(2f) else 0, c)
                        }
                        flow.addView(chip)
                        register(chip) { a.put("name", n); next() }
                    }
                    place(flow)
                    val f = field("Or type a name", typedName, maxLength = 20)
                    f.addTextChangedListener(watch { typedName = it })
                    button("Use this name") {
                        val t = f.text.toString().trim()
                        if (t.isNotEmpty()) { a.put("name", t); next() }
                    }
                }
                "role" -> {
                    titleRow(a, "What's $name for?")
                    for ((id, label) in ROLES) {
                        card(choice(label, null, a.optString("role") == id, c)) { a.put("role", id); next() }
                    }
                    val f = field("Or say it in a few words", typedRole, maxLength = 120)
                    f.addTextChangedListener(watch { typedRole = it })
                    button("Use what I typed", primary = false) {
                        val t = f.text.toString().trim()
                        if (t.isNotEmpty()) { a.put("role", t); next() }
                    }
                }
                else -> {
                    titleRow(a, "How should $name talk?")
                    for ((id, label, hint) in PERSONALITIES) {
                        if (saving) place(choice(label, hint, a.optString("personality") == id, c).apply { alpha = 0.5f })
                        else card(choice(label, hint, a.optString("personality") == id, c)) { a.put("personality", id); save() }
                    }
                    err?.let { note(it, Theme.BAD, 13f) }
                    note(if (saving) "Saving…" else "Click one to $doneLabel", size = 12f, center = true)
                }
            }
        }
        // A new step starts on the agent's current choice (its name, role or voice), not the first one.
        if (fresh) {
            val at = when (step) {
                "name" -> (if (name in NAMES) NAMES else listOf(name) + NAMES).take(8).indexOf(name)
                "role" -> ROLES.indexOfFirst { it.first == a.optString("role") }
                else -> PERSONALITIES.indexOfFirst { it.first == a.optString("personality") }
            }
            if (at > 0) list.setFocus(at, animate = false)
        }
    }

    /** A `.row` choice: a label (and a hint), ringed in the colour when it's the current one. */
    private fun choice(label: String, hint: String?, on: Boolean, c: Int) = LinearLayout(context).apply {
        orientation = LinearLayout.HORIZONTAL
        gravity = Gravity.CENTER_VERTICAL
        setPadding(context.dpi(12f), context.dpi(12f), context.dpi(12f), context.dpi(12f))
        background = roundRect(Theme.PANEL, context.dp(14f), if (on) context.dpi(2f) else 0, c)
        addView(context.text(18f, if (hint != null) 700 else 500).apply { text = label })
        if (hint != null) addView(context.text(13f, 500, Theme.MUTED).apply { text = " · $hint"; maxLines = 1; ellipsize = android.text.TextUtils.TruncateAt.END })
    }

    companion object {
        /** The colour step's last choice: a colour of your own (T-060). */
        private const val OWN = "own"
        /** A step per place it wears things (the wardrobe's slots: head, face, cheeks, neck, back). */
        private val WEAR_STEPS: List<String> get() = Wardrobe.catalog.slots.map { it.id }
        private val PICKS: Set<String> get() = setOf("form", "color", "finish", "eyes") + WEAR_STEPS
        /** The designer's steps, in order: the look (picked on the wheel, with a live preview), then who it is. */
        private val STEPS: List<String> get() = listOf("form", "color", "finish", "eyes") + WEAR_STEPS + listOf("name", "role", "personality")
        val FINISH_LABEL = mapOf("soft" to "Soft", "matte" to "Matte", "grain" to "Grain", "noise" to "Noise", "gradient" to "Gradient", "grainy" to "Grainy gradient", "aurora" to "Aurora")
        val NAMES = listOf("Pip", "Nova", "Mochi", "Echo", "Juniper", "Bolt", "Kiwi", "Sol")
        val ROLES = listOf(
            "" to "A bit of everything", "apps and tools" to "Apps and tools", "plans, lists and reminders" to "Plans and lists",
            "research and the news" to "Research and news", "health, fitness and habits" to "Health and habits", "someone to talk to" to "Someone to talk to",
        )
        val PERSONALITIES = listOf(
            Triple("friendly", "Friendly", "Warm and encouraging"), Triple("concise", "Concise", "Straight to the point"),
            Triple("playful", "Playful", "Curious, a bit of wit"), Triple("calm", "Calm", "Unhurried and steady"),
        )

        /** A fresh look nobody has yet: the next unused colour, name and form (New agent). */
        fun freshAgent(taken: List<dev.r1os.host.net.Agent>): JSONObject {
            val colors = taken.map { it.color }.toSet()
            val names = taken.map { it.name }.toSet()
            return JSONObject()
                .put("name", NAMES.firstOrNull { it !in names } ?: "Nova")
                .put("form", CharacterView.FORMS[taken.size % CharacterView.FORMS.size])
                .put("color", Theme.COLORS.keys.firstOrNull { it !in colors } ?: "blue")
                .put("finish", "soft").put("personality", "friendly").put("custom", "").put("role", "").put("brain", "claude")
        }
    }
}

private fun watch(fn: (String) -> Unit) = object : android.text.TextWatcher {
    override fun beforeTextChanged(s: CharSequence?, start: Int, count: Int, after: Int) {}
    override fun onTextChanged(s: CharSequence?, start: Int, before: Int, count: Int) {}
    override fun afterTextChanged(s: android.text.Editable?) = fn(s?.toString().orEmpty())
}

/** Children left to right, wrapping onto new lines (the name chips). */
@SuppressLint("ViewConstructor")
class Flow(context: Context, private val gap: Int = context.dpi(8f)) : ViewGroup(context) {
    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        val max = MeasureSpec.getSize(widthMeasureSpec)
        var x = 0
        var y = 0
        var line = 0
        for (i in 0 until childCount) {
            val c = getChildAt(i)
            c.measure(MeasureSpec.makeMeasureSpec(max, MeasureSpec.AT_MOST), MeasureSpec.UNSPECIFIED)
            if (x > 0 && x + c.measuredWidth > max) { x = 0; y += line + gap; line = 0 }
            x += c.measuredWidth + gap
            line = maxOf(line, c.measuredHeight)
        }
        setMeasuredDimension(max, y + line)
    }

    override fun onLayout(changed: Boolean, l: Int, t: Int, r: Int, b: Int) {
        val max = r - l
        var x = 0
        var y = 0
        var line = 0
        for (i in 0 until childCount) {
            val c = getChildAt(i)
            if (x > 0 && x + c.measuredWidth > max) { x = 0; y += line + gap; line = 0 }
            c.layout(x, y, x + c.measuredWidth, y + c.measuredHeight)
            x += c.measuredWidth + gap
            line = maxOf(line, c.measuredHeight)
        }
    }
}

// ---- Connect a brain ------------------------------------------------------------------------------

/**
 * Sign in to Claude or Codex for every agent (ADR-0004): r1d starts the CLI's login and this shows
 * its QR code (and Codex's code) until the account event says it's connected.
 */
class BrainPanel(shell: Shell, private val agent: JSONObject, private val doneLabel: String = "Continue", private val onDone: () -> Unit) :
    ListPanel(shell, Theme.colorOf(agent.optString("color"))) {
    private var acc: JSONObject? = null
    private var err: String? = null
    private var starting: String? = null
    /** The sign-in on screen: brain, and its `pair_url` (Claude) or `url` + `code` (Codex), and state. */
    private var pairing: JSONObject? = null
    private var drawnMode = ""

    init {
        load()
        render()
    }

    private fun load(): Unit = R1d.get("/api/accounts", onError = { err = it; render() }) { acc = it.optJSONObject("accounts"); render() }

    override fun onAnyEvent(ev: Ev) {
        if (ev.type != "account") return
        val p = pairing
        val state = ev.str("state") ?: return
        if (p != null && p.optString("brain") == ev.str("brain")) p.put("state", state)
        if (state == "signed_in" || state == "signed_out") {
            load()
            if (state == "signed_in") R1d.main.postDelayed({ if (pairing?.optString("brain") == ev.str("brain")) { pairing = null; render() } }, 1500)
        }
        render()
    }

    override fun onBack(): Boolean {
        if (pairing == null) return false
        pairing = null
        render()
        return true
    }

    private fun start(brain: String) {
        err = null
        starting = brain
        render()
        R1d.post("/api/accounts/$brain/login", onError = { err = it; starting = null; render() }) { r ->
            starting = null
            pairing = if (brain == "claude") JSONObject().put("brain", brain).put("qr", r.optString("pair_url")).put("state", "waiting")
            else JSONObject().put("brain", brain).put("qr", r.optString("url")).put("code", r.optString("code")).put("state", "waiting")
            render()
        }
    }

    private fun render() {
        val p = pairing
        val mode = p?.optString("brain") ?: "cards"
        if (mode != drawnMode) { list.clear(); drawnMode = mode }
        list.rebuild {
            if (p != null) {
                val claude = p.optString("brain") == "claude"
                val state = p.optString("state")
                val done = state == "signed_in"
                note(if (claude) "Connect Claude" else "Connect Codex", Theme.FG, 22f, center = true, weight = 750)
                if (done) {
                    place(FrameLayout(context).apply {
                        minimumWidth = context.dpi(140f)
                        minimumHeight = context.dpi(140f)
                        background = android.graphics.drawable.GradientDrawable().apply {
                            shape = android.graphics.drawable.GradientDrawable.OVAL
                            setColor(Theme.withAlpha(Theme.OK, 0.2f))
                        }
                        addView(ImageView(context).apply { setImageDrawable(Icons.drawable("check", Theme.OK)) }, FrameLayout.LayoutParams(context.dpi(64f), context.dpi(64f), Gravity.CENTER))
                    }, matchWidth = false, gravity = Gravity.CENTER_HORIZONTAL)
                } else {
                    val qr = QrView(context, p.optString("qr"))
                    place(FrameLayout(context).apply { addView(qr, FrameLayout.LayoutParams(context.dpi(190f), context.dpi(190f), Gravity.CENTER)) })
                }
                if (!claude && !done) note(p.optString("code"), Theme.FG, 30f, center = true, weight = 800).apply { letterSpacing = 0.08f }
                val text = when (state) {
                    "waiting" -> if (claude) "Scan with your phone's camera" else "Approve on your phone"
                    "code_entered" -> "Checking the code…"
                    "signed_in" -> "Connected"
                    "failed" -> "That didn't work. Try again."
                    "expired" -> "This code expired. Try again."
                    else -> state
                }
                note(text, if (state == "failed" || state == "expired") Theme.BAD else Theme.FG, 16f, center = true, weight = 600)
                if (claude && !done) note("Your phone must be on the same Wi-Fi.", size = 13f, center = true)
                button("Back", primary = false) { pairing = null; render() }
                return@rebuild
            }
            titleRow(agent, "Give ${agent.optString("name")} a brain", 64f, if (acc == null) "thinking" else "idle")
            if (acc == null && err == null) note("Checking accounts…")
            for ((brain, label, sub) in listOf(Triple("claude", "Claude", "Sign in with your Claude account"), Triple("codex", "Codex", "Sign in with ChatGPT"))) {
                val on = acc?.optBoolean(brain) == true
                val v = context.card(
                    context.text(20f, 750).apply { text = label },
                    context.text(15f, 500, Theme.MUTED).apply { text = if (on) "Connected ✓" else if (starting == brain) "Starting…" else sub },
                    ok = on,
                )
                card(v) { if (!on && starting == null) start(brain) }
            }
            err?.let { note(it, Theme.BAD, 13f) }
            val any = acc?.optBoolean("claude") == true || acc?.optBoolean("codex") == true || acc?.optBoolean("hermes") == true
            button(doneLabel, enabled = any) { onDone() }
            if (acc != null && !any) button("Set up later", primary = false) { onDone() }
        }
    }
}
