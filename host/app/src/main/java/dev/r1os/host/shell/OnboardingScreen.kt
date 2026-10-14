package dev.r1os.host.shell

import android.graphics.Color
import android.view.Gravity
import android.view.View
import android.widget.FrameLayout
import android.widget.LinearLayout
import dev.r1os.host.net.Agent
import dev.r1os.host.net.Ev
import dev.r1os.host.net.R1d
import dev.r1os.host.ui.CharacterView
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dp
import dev.r1os.host.ui.dpi
import dev.r1os.host.ui.text
import org.json.JSONObject

/**
 * First run (docs/specs/experience.md §2.4; ported from shell/src/onboarding.tsx):
 * hello → Wi-Fi → make your agent → connect a brain → ready. Every step is saved in r1d, so a reboot
 * resumes where you were. There's no back from here: double tap only backs out of a sub-step.
 */
class OnboardingScreen(shell: Shell, start: String) : Screen(shell) {
    private var step = if (start in ORDER) start else "hello"
    private var panel: Panel? = null
    private var stepView: View? = null
    private val dots = LinearLayout(context).apply { orientation = LinearLayout.HORIZONTAL; gravity = Gravity.CENTER }
    private val content = FrameLayout(context)
    private var shown = false
    private var drawnLook = ""
    override val view = LinearLayout(context).apply {
        orientation = LinearLayout.VERTICAL
        setBackgroundColor(Theme.BG)
        addView(dots, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(10f); bottomMargin = context.dpi(4f) })
        addView(content, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, 0, 1f))
    }

    /** The agent being set up: r1d always has one. */
    private val agent: Agent get() = shell.agents.firstOrNull() ?: Agent.DEFAULT

    init { show(animate = false) }

    private fun go(s: String) {
        R1d.put("/api/onboarding", JSONObject().put("step", s))
        if (s == "done") {
            shell.setupDone()
            return
        }
        step = s
        show(animate = true)
    }

    private fun next() = go(ORDER.getOrNull(ORDER.indexOf(step) + 1) ?: "done")

    private fun show(animate: Boolean) {
        panel?.let { it.onHide(); it.onDestroy() }
        panel = null
        stepView?.let { content.removeView(it) }
        val a = agent
        drawnLook = listOf(a.id, a.name, a.form, a.color, a.finish).joinToString()
        val accent = Theme.colorOf(a.color)
        // Progress: the steps after hello.
        dots.removeAllViews()
        dots.visibility = if (step == "hello") View.INVISIBLE else View.VISIBLE
        ORDER.drop(1).forEachIndexed { i, _ ->
            dots.addView(View(context).apply { background = roundRect(if (i + 1 <= ORDER.indexOf(step)) accent else Theme.SUBTLE, context.dp(2f)) },
                LinearLayout.LayoutParams(context.dpi(22f), context.dpi(4f)).apply { setMargins(context.dpi(3f), 0, context.dpi(3f), 0) })
        }
        val v: View = when (step) {
            "wifi" -> WifiPanel(shell) { next() }.also { panel = it }.view
            "agent" -> DesignerPanel(shell, a.raw.takeIf { it.length() > 0 } ?: JSONObject().put("name", a.name).put("form", a.form).put("color", a.color).put("finish", a.finish), "Continue",
                onSave = { x, ok, fail -> R1d.put("/api/agents/${R1d.enc(a.id)}", x, onError = fail) { ok(it.optJSONObject("agent") ?: x) } },
                // The next steps show the agent as made: once the list has it.
                onDone = { shell.reloadAgents { next() } },
            ).also { panel = it }.view
            "brain" -> BrainPanel(shell, a.raw.takeIf { it.length() > 0 } ?: JSONObject().put("name", a.name).put("color", a.color), "Continue") { next() }.also { panel = it }.view
            "ready" -> hello(a, "I'm ${a.name}.", "Hold the button to talk. Ask for anything, or ask me to make you an app.",
                "Click to chat, double-click for my chats and memory. Turn down past your agents for your apps, up past them to sleep.", "press to start", "speaking", hatch = false)
            else -> hello(a, "Hi.", "Let's set me up. It takes a minute.", null, "press the side button to start", "idle", hatch = true)
        }
        stepView = v
        content.addView(v, FrameLayout.LayoutParams(FrameLayout.LayoutParams.MATCH_PARENT, FrameLayout.LayoutParams.MATCH_PARENT))
        if (animate) {
            v.alpha = 0f
            v.translationX = context.dp(24f)
            v.animate().alpha(1f).translationX(0f).setDuration(320).setInterpolator(Theme.SETTLE).start()
        }
        if (shown) panel?.onShow()
    }

    /** Hello and Ready: the agent, big, and a few words. Hello hatches it. */
    private fun hello(a: Agent, title: String, line: String, small: String?, hint: String, state: String, hatch: Boolean): View {
        val ch = CharacterView(context, context.dpi(if (hatch) 190f else 170f))
        ch.set(a.form, a.color, a.finish, if (hatch) "sleeping" else state, eyes = a.eyes, wear = a.wear)
        val texts = LinearLayout(context).apply {
            orientation = LinearLayout.VERTICAL
            gravity = Gravity.CENTER_HORIZONTAL
            addView(context.text(30f, 800).apply { text = title; gravity = Gravity.CENTER })
            addView(context.text(16f, 500, Theme.MUTED).apply { text = line; gravity = Gravity.CENTER; setPadding(0, context.dpi(8f), 0, 0) })
            if (small != null) addView(context.text(13f, 500, Theme.MUTED).apply { text = small; gravity = Gravity.CENTER; setPadding(0, context.dpi(8f), 0, 0) })
        }
        val hintView = context.text(12f, 500, Theme.MUTED).apply { text = hint; gravity = Gravity.CENTER }
        val col = LinearLayout(context).apply {
            orientation = LinearLayout.VERTICAL
            gravity = Gravity.CENTER
            clipChildren = false
            setPadding(context.dpi(20f), 0, context.dpi(20f), context.dpi(24f))
            addView(ch, LinearLayout.LayoutParams(ch.layoutParams.width, ch.layoutParams.height))
            addView(texts, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(14f) })
            addView(hintView, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(14f) })
            setOnClickListener { next() }
        }
        if (hatch) {
            // It hatches: the character pops in, then the words fade up.
            ch.alpha = 0f; ch.scaleX = 0.6f; ch.scaleY = 0.6f
            texts.alpha = 0f; hintView.alpha = 0f
            col.postDelayed({
                ch.set(a.form, a.color, a.finish, "idle", eyes = a.eyes, wear = a.wear)
                ch.animate().alpha(1f).setDuration(700).start()
                ch.animate().scaleX(1f).scaleY(1f).setDuration(900).setInterpolator(Theme.POP).start()
                texts.animate().alpha(1f).setStartDelay(500).setDuration(600).start()
                hintView.animate().alpha(1f).setStartDelay(500).setDuration(600).start()
            }, 400)
        }
        return col
    }

    override fun onEvent(ev: Ev): Boolean {
        val p = panel
        if (p != null) {
            if (ev.isDoubleTap) return p.onBack() || true
            return p.onInput(ev)
        }
        // Hello and Ready: a click goes on; the wheel does nothing.
        if (ev.isTap) { next(); return true }
        return ev.type == "wheel" || ev.isDoubleTap
    }

    override fun onAnyEvent(ev: Ev) { panel?.onAnyEvent(ev) }

    /** First boot: the agents can arrive after this opened. Hello and Ready redraw with its look. */
    override fun onAgents() {
        val a = agent
        val look = listOf(a.id, a.name, a.form, a.color, a.finish).joinToString()
        if (look == drawnLook) return
        // Drawn before r1d's agents arrived (a placeholder with no id): the step is drawn again with the real
        // one, even mid-panel, since nothing could be saved to the placeholder anyway.
        val placeholder = drawnLook.startsWith(",")
        drawnLook = look
        if (panel == null || placeholder) show(animate = false)
    }


    override fun onShow() {
        shown = true
        panel?.onShow()
    }

    override fun onHide() {
        shown = false
        panel?.onHide()
    }

    override fun onDestroy() {
        panel?.onDestroy()
    }

    private companion object {
        val ORDER = listOf("hello", "wifi", "agent", "brain", "ready")
    }
}
