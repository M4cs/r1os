package dev.r1os.host.shell

import android.view.Gravity
import android.widget.LinearLayout
import dev.r1os.host.net.R1d
import dev.r1os.host.ui.CharacterView
import dev.r1os.host.ui.Theme
import dev.r1os.host.ui.dpi
import dev.r1os.host.ui.text
import org.json.JSONObject

/**
 * An agent asks to change something on the R1 (ADR-0045): its look or home, its settings, or an app's settings or
 * permissions. r1d sends `approval_asked` (waking the display) and the agent's tool waits for the answer here:
 * **allow**, **always allow** (it won't ask about that area again; not offered for granting an app a permission), or
 * **don't allow**. Leaving without an answer is a no. If nobody answers in time, r1d says so (`approval_done`) and the
 * page goes.
 *
 * Nothing is focused at first, so a stray click doesn't answer: the wheel picks. Or hold and say it (T-055): "allow",
 * "always" or "don't" (and what they mean: "go ahead", "don't ask me again", "not now"), which System One hears in r1d.
 */
class ApprovalScreen(shell: Shell, private val approval: JSONObject) :
    ListPage(shell, "Allow this?", Theme.colorOf(shell.agent(approval.optString("agent"))?.color)) {
    val approvalId: String = approval.optString("id")
    private var answered = false
    /** The card offers "always" (not for granting an app a permission). */
    val offersAlways = approval.optBoolean("always", true)

    /** Holding to talk here shows the asking agent. */
    override val agentId: String? = approval.optString("agent").ifEmpty { null }

    init {
        // The display stays on while you decide (r1d waits two minutes at most).
        view.keepScreenOn = true
        render()
        list.blur()
    }

    /** The R1 went to sleep: the page goes, and the request keeps waiting in r1d (it shows again on waking). */
    fun putAway() {
        answered = true
    }

    private fun render(): Unit = list.rebuild {
        val a = shell.agent(approval.optString("agent"))
        val name = approval.optString("name").ifEmpty { a?.name ?: "An agent" }
        place(LinearLayout(context).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            clipChildren = false
            setPadding(context.dpi(4f), 0, 0, context.dpi(2f))
            if (a != null) {
                addView(CharacterView(context, context.dpi(52f)).apply { set(a.form, a.color, a.finish, "idle", eyes = a.eyes, wear = a.wear) },
                    LinearLayout.LayoutParams(context.dpi(52f), context.dpi(52f)).apply { rightMargin = context.dpi(12f) })
            }
            addView(context.text(18f, 650, Theme.MUTED).apply { text = "$name wants to" })
        })
        val title = approval.optString("title")
        place(context.text(if (title.length > 44) 20f else 24f, 750).apply {
            text = title
            setLineSpacing(0f, 1.05f)
            setPadding(context.dpi(6f), 0, context.dpi(6f), 0)
        })
        // A final step's order, read from the app's screen (T-113, Q57): its rows; else the screen's words as they are.
        val order = approval.optJSONObject("order")
        if (order != null) placeOrder(order) else approval.optString("detail").takeIf { it.isNotBlank() }?.let { note(it, color = Theme.FG, size = 17f) }
        approval.optString("why").takeIf { it.isNotBlank() }?.let { note("“$it”", size = 15f) }
        item("check", "allow", accent) { answer(allow = true, always = false) }
        if (approval.optBoolean("always", true)) {
            item("check-circle", "always allow", sub = "$name won't ask about ${approval.optString("area_words")} again") {
                answer(allow = true, always = true)
            }
        }
        item("x", "don't allow") { answer(allow = false, always = false) }
        note(if (offersAlways) "or hold and say “allow”, “always” or “don't”" else "or hold and say “allow” or “don't”", size = 14f)
    }

    /**
     * What a final step would order (T-113): each item and its price, the tip, the total (big), where it goes, how it's
     * paid and when, in one block. Read by r1d from the app's screen, not the agent's word.
     */
    private fun FocusList.placeOrder(o: JSONObject) {
        val box = LinearLayout(context).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(context.dpi(14f), context.dpi(10f), context.dpi(14f), context.dpi(12f))
            background = roundRect(Theme.CARD, context.dpi(16f).toFloat())
        }
        fun row(name: String, price: String?, size: Float = 17f, weight: Int = 600, color: Int = Theme.FG) {
            box.addView(LinearLayout(context).apply {
                orientation = LinearLayout.HORIZONTAL
                gravity = Gravity.CENTER_VERTICAL
                addView(context.text(size, weight, color).apply { text = name; maxLines = 2 }, LinearLayout.LayoutParams(0, LinearLayout.LayoutParams.WRAP_CONTENT, 1f))
                if (price != null) addView(context.text(size, weight, color).apply { text = price; setPadding(context.dpi(10f), 0, 0, 0) })
            }, LinearLayout.LayoutParams(LinearLayout.LayoutParams.MATCH_PARENT, LinearLayout.LayoutParams.WRAP_CONTENT).apply { topMargin = context.dpi(4f) })
        }
        // A field r1d didn't find is a JSON null, which optString reads as "null": the card said "tip null" (2026-10-13).
        fun JSONObject.word(k: String) = if (isNull(k)) null else optString(k).ifEmpty { null }
        val items = o.optJSONArray("items")
        val shown = minOf(items?.length() ?: 0, ORDER_ITEMS)
        for (i in 0 until shown) {
            val it = items!!.optJSONObject(i) ?: continue
            row(it.word("name") ?: continue, it.word("price"))
        }
        val more = (items?.length() ?: 0) - shown
        if (more > 0) row("+$more more", null, size = 15f, weight = 500, color = Theme.MUTED)
        o.word("tip")?.let { row("tip", it, size = 15f, weight = 500, color = Theme.MUTED) }
        o.word("total")?.let { row("total", it, size = 22f, weight = 800) }
        o.word("to")?.let { row("to $it", null, size = 15f, weight = 500, color = Theme.MUTED) }
        o.word("pay")?.let { row("with $it", null, size = 15f, weight = 500, color = Theme.MUTED) }
        o.word("when")?.let { row(it, null, size = 15f, weight = 500, color = Theme.MUTED) }
        place(box)
    }

    /**
     * What you said, held on the card (T-055): System One (`POST /api/approvals/heard`) says which answer it is, and it's
     * answered as if picked. `heard(false)`: it wasn't one ("always" where there's no always is a yes, this time).
     */
    fun say(words: String, heard: (Boolean) -> Unit) {
        if (answered) return
        R1d.post("/api/approvals/heard", JSONObject().put("text", words), onError = { heard(false) }) { r ->
            when (r.optString("answer")) {
                "allow" -> { heard(true); answer(allow = true, always = false) }
                "always" -> { heard(true); answer(allow = true, always = offersAlways) }
                "deny" -> { heard(true); answer(allow = false, always = false) }
                else -> heard(false)
            }
        }
    }

    private fun answer(allow: Boolean, always: Boolean) {
        if (answered) return
        answered = true
        R1d.post("/api/approvals/${R1d.enc(approvalId)}", JSONObject().put("allow", allow).put("always", always))
        shell.flash(if (allow) "allowed" else "not allowed")
        shell.back()
    }

    /** r1d stopped waiting (answered elsewhere, or nobody answered in time): the page goes, without answering. */
    fun done() {
        answered = true
        shell.remove(this)
    }

    override fun onDestroy() {
        if (!answered) R1d.post("/api/approvals/${R1d.enc(approvalId)}", JSONObject().put("allow", false))
    }

    private companion object {
        /** An order's items on the card; the rest as "+N more". */
        const val ORDER_ITEMS = 5
    }
}
