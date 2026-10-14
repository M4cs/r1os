package dev.r1os.host.ime

import android.content.Context
import android.content.pm.ActivityInfo
import android.inputmethodservice.InputMethodService
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.text.InputType
import android.view.KeyEvent
import android.view.Surface
import android.view.View
import android.view.inputmethod.EditorInfo

/**
 * r1os's keyboard (ADR-0055; Max, 2026-10-11: "When we go to a keyboard, we should rotate the screen, and show a nice
 * custom keyboard that matches our design aesthetic. This should go for all keyboards on the OS and all text boxes."):
 * Android's input method for every text box on the R1, r1os's own and other apps'.
 *
 * - **On its side while it's up:** the R1 turns to landscape when a field wants typing (the system's rotation, and
 *   r1os's own screen), and back to upright a moment after the keyboard goes.
 * - **The R1's own controls:** the wheel moves the cursor, a click is the field's action (go, search, send, next,
 *   done), a double click puts the keyboard away, and holding the button types what you say (the shell's
 *   speech-to-text, not an agent). Over another app r1d sends the wheel and the click as D-pad keys, which the
 *   keyboard takes first; over r1os the shell hands them over ([current]).
 */
class R1Keyboard : InputMethodService() {
    private var view: KeyboardView? = null
    private val main = Handler(Looper.getMainLooper())
    private var editor: EditorInfo? = null

    /** The keyboard is on screen. */
    val shown: Boolean get() = isInputViewShown

    override fun onCreate() {
        super.onCreate()
        current = this
        // Its fonts and the look's colours, in case r1os's own screen hasn't loaded them in this process yet.
        runCatching { dev.r1os.host.ui.Theme.init(this); dev.r1os.host.ui.Look.load(this) }
    }

    override fun onDestroy() {
        if (current === this) current = null
        Rotation.set(this, false)
        super.onDestroy()
    }

    override fun onCreateInputView(): View = KeyboardView(this, this).also { view = it; editor?.let { e -> fit(e) } }

    /** Always the full keyboard, never Android's full-screen editing on its side. */
    override fun onEvaluateFullscreenMode(): Boolean = false

    override fun onStartInputView(info: EditorInfo, restarting: Boolean) {
        super.onStartInputView(info, restarting)
        editor = info
        main.removeCallbacks(upright)
        Rotation.set(this, true)
        fit(info)
    }

    override fun onFinishInputView(finishingInput: Boolean) {
        super.onFinishInputView(finishingInput)
        // A moment's grace: going from one field to the next shouldn't turn the R1 back and forth.
        main.removeCallbacks(upright)
        main.postDelayed(upright, UPRIGHT_MS)
    }

    private val upright = Runnable { if (!isInputViewShown) Rotation.set(this, false) }

    /** The field's own words, action, and numbers or letters first. */
    private fun fit(info: EditorInfo) {
        val v = view ?: return
        val cls = info.inputType and InputType.TYPE_MASK_CLASS
        v.start(numbers = cls == InputType.TYPE_CLASS_NUMBER || cls == InputType.TYPE_CLASS_PHONE)
        v.action = actionLabel(info)
        val variation = info.inputType and InputType.TYPE_MASK_VARIATION
        val password = cls == InputType.TYPE_CLASS_TEXT && variation in setOf(
            InputType.TYPE_TEXT_VARIATION_PASSWORD, InputType.TYPE_TEXT_VARIATION_WEB_PASSWORD, InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD,
        )
        val words = (info.hintText?.toString()?.takeIf { it.isNotBlank() } ?: info.label?.toString()).orEmpty().lowercase()
        v.hint = if (password && !words.contains("password")) listOf(words, "password").filter { it.isNotEmpty() }.joinToString(" · ") else words
        capsForHere()
    }

    private fun actionLabel(info: EditorInfo): String {
        if (info.imeOptions and EditorInfo.IME_FLAG_NO_ENTER_ACTION != 0) return "↵"
        return when (info.imeOptions and EditorInfo.IME_MASK_ACTION) {
            EditorInfo.IME_ACTION_GO -> "go"
            EditorInfo.IME_ACTION_SEARCH -> "search"
            EditorInfo.IME_ACTION_SEND -> "send"
            EditorInfo.IME_ACTION_NEXT -> "next"
            EditorInfo.IME_ACTION_DONE -> "done"
            EditorInfo.IME_ACTION_PREVIOUS -> "back"
            else -> "↵"
        }
    }

    /** The start of a sentence (where the field wants capitals): shift for the next letter. */
    private fun capsForHere() {
        val v = view ?: return
        if (v.shift == 2) return
        val caps = currentInputConnection?.getCursorCapsMode(editor?.inputType ?: 0) ?: 0
        v.shift = if (caps != 0) 1 else 0
    }

    // ---- typing ---------------------------------------------------------------------------------------------

    fun commit(text: String) {
        currentInputConnection?.commitText(text, 1)
        if (text == " " || text == "." || text == "?" || text == "!") capsForHere()
    }

    fun backspace() {
        val ic = currentInputConnection ?: return
        val selected = ic.getSelectedText(0)
        if (!selected.isNullOrEmpty()) ic.commitText("", 1) else sendDownUpKeyEvents(KeyEvent.KEYCODE_DEL)
        capsForHere()
    }

    /** The big key, or a click: the field's action, else a new line. */
    fun action() {
        val info = editor ?: return
        val act = info.imeOptions and EditorInfo.IME_MASK_ACTION
        val multi = info.inputType and InputType.TYPE_TEXT_FLAG_MULTI_LINE != 0
        if (info.imeOptions and EditorInfo.IME_FLAG_NO_ENTER_ACTION != 0 || act == EditorInfo.IME_ACTION_NONE || act == EditorInfo.IME_ACTION_UNSPECIFIED || multi) {
            if (multi) commit("\n") else sendDownUpKeyEvents(KeyEvent.KEYCODE_ENTER)
        } else {
            currentInputConnection?.performEditorAction(act)
        }
    }

    /** The wheel: the cursor, a character a tick. */
    fun moveCursor(ticks: Int) {
        val key = if (ticks < 0) KeyEvent.KEYCODE_DPAD_LEFT else KeyEvent.KEYCODE_DPAD_RIGHT
        repeat(minOf(kotlin.math.abs(ticks), 20)) { sendDownUpKeyEvents(key) }
    }

    /** What you said while holding the button, typed where the cursor is (a space before it if it needs one). */
    fun dictate(text: String) {
        val ic = currentInputConnection ?: return
        val before = ic.getTextBeforeCursor(1, 0)?.toString().orEmpty()
        val words = text.trim()
        if (words.isEmpty()) return
        ic.commitText(if (before.isNotEmpty() && !before[0].isWhitespace()) " $words" else words, 1)
    }

    fun hideKeyboard() = requestHideSelf(0)

    // ---- the R1's wheel and click over other apps: r1d sends them as D-pad keys ----------------------------

    override fun onKeyDown(keyCode: Int, event: KeyEvent): Boolean {
        if (!isInputViewShown) return super.onKeyDown(keyCode, event)
        when (keyCode) {
            KeyEvent.KEYCODE_DPAD_UP, KeyEvent.KEYCODE_DPAD_LEFT -> moveCursor(-1)
            KeyEvent.KEYCODE_DPAD_DOWN, KeyEvent.KEYCODE_DPAD_RIGHT -> moveCursor(1)
            KeyEvent.KEYCODE_DPAD_CENTER -> action()
            else -> return super.onKeyDown(keyCode, event)
        }
        return true
    }

    override fun onKeyUp(keyCode: Int, event: KeyEvent): Boolean =
        if (isInputViewShown && keyCode in setOf(KeyEvent.KEYCODE_DPAD_UP, KeyEvent.KEYCODE_DPAD_DOWN, KeyEvent.KEYCODE_DPAD_LEFT, KeyEvent.KEYCODE_DPAD_RIGHT, KeyEvent.KEYCODE_DPAD_CENTER)) true
        else super.onKeyUp(keyCode, event)

    companion object {
        /** The running keyboard, for the shell (the R1's controls over r1os's own screens, and dictation). */
        var current: R1Keyboard? = null
            private set
        const val UPRIGHT_MS = 450L
    }
}

/**
 * The R1 on its side while you type (ADR-0055): the system's rotation for other apps (WRITE_SETTINGS), and r1os's own
 * screen asks for landscape too (its activity is upright otherwise). Two asks: the keyboard's ([set]: its left side
 * down), and the R1 held on a side by its tilt ([hold]: docked, T-103; a chat, T-089). A held side wins, so the keyboard
 * that comes up in a chat on its side stays on the side you're holding.
 */
object Rotation {
    /** The keyboard's: it's up. */
    private var ime = false
    /** The tilt's: 1 its right side down, -1 its left, 0 none. */
    private var held = 0
    private var applied: Int? = null
    /** r1os's own activity, while it's up: it turns with the keyboard. */
    var activity: android.app.Activity? = null

    /** The keyboard came up (on) or went (off). */
    fun set(context: Context, on: Boolean) {
        ime = on
        apply(context)
    }

    /** Docked on its side (T-103): its right side at the bottom, so the other landscape from the keyboard's. */
    fun dock(context: Context, on: Boolean) = hold(context, if (on) 1 else 0)

    /** Held on `side` by the R1's tilt (1 its right side down, -1 its left), or upright (0): a keyboard it brought goes with it. */
    fun hold(context: Context, side: Int) {
        if (side == 0 && held != 0) ime = false
        held = side
        apply(context)
    }

    private fun apply(context: Context) {
        val side = if (held != 0) held else if (ime) -1 else 0
        if (side == applied) return
        applied = side
        runCatching {
            val cr = context.contentResolver
            if (Settings.System.canWrite(context)) {
                Settings.System.putInt(cr, Settings.System.ACCELEROMETER_ROTATION, 0)
                Settings.System.putInt(cr, Settings.System.USER_ROTATION, when (side) {
                    1 -> Surface.ROTATION_270
                    -1 -> Surface.ROTATION_90
                    else -> Surface.ROTATION_0
                })
            }
        }
        activity?.requestedOrientation = when (side) {
            1 -> ActivityInfo.SCREEN_ORIENTATION_REVERSE_LANDSCAPE
            -1 -> ActivityInfo.SCREEN_ORIENTATION_LANDSCAPE
            else -> ActivityInfo.SCREEN_ORIENTATION_PORTRAIT
        }
    }
}
