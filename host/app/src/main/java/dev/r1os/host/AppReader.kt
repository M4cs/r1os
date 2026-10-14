package dev.r1os.host

import android.accessibilityservice.AccessibilityService
import android.view.accessibility.AccessibilityEvent

/**
 * The app window's reader (ADR-0061): an accessibility service, so [AgentApps] can read the screen of the app an agent
 * drives as text. It handles no events and reads only when asked; [AgentApps] turns it on when an app window opens and
 * off when it closes, since every app does accessibility work while any service runs (ADR-0027).
 */
class AppReader : AccessibilityService() {
    override fun onServiceConnected() {
        instance = this
        AgentApps.readerConnected()
    }

    override fun onAccessibilityEvent(event: AccessibilityEvent?) {}

    override fun onInterrupt() {}

    override fun onUnbind(intent: android.content.Intent?): Boolean {
        if (instance === this) instance = null
        return super.onUnbind(intent)
    }

    override fun onDestroy() {
        if (instance === this) instance = null
        super.onDestroy()
    }

    companion object {
        /** The running service, while it's on. */
        @Volatile
        var instance: AppReader? = null
            private set
    }
}
