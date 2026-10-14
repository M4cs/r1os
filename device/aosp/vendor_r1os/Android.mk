LOCAL_PATH := $(call my-dir)

# The keep-list's first cut (roadmap R4, Q22; image v0.2.9): system apps nothing in r1os uses and the keep-list
# doesn't name. Kept on purpose: LTE data's plumbing (Telecom, TeleService, MmsService, the telephony providers,
# ONS, Iwlan, CarrierConfig), what Android apps need (contacts and calendar providers, CredentialManager,
# DocumentsUI, the installer, the keyboard, WebView), Shell (adb), Provision (marks a wiped device set up) and
# TrebleApp (TrebleDroid's hardware fixes). Settings and SystemUI go later, each once r1os replaces what we use.
R1OS_CUT_1 := \
    CellBroadcastLegacyApp SimAppDialog MtkInCallService CallLogBackup WAPPushManager \
    PrintSpooler PrintRecommendationService BuiltInPrintService \
    Tag \
    WallpaperBackup WallpaperCropper \
    AvatarPicker DeviceAsWebcam DeviceDiagnostics DynamicSystemInstallationService EmergencyInfo \
    AccessibilityMenu BookmarkProvider HTMLViewer BluetoothMidiService CameraExtensionsProxy QcRilAm \
    ImsServiceEntitlement ManagedProvisioning E2eeContactKeysProvider SettingsIntelligence

# The host app (host/, ADR-0002, ADR-0017): HOME, the native shell, and the framework-only I/O.
# tools/build-image.sh copies out/r1os-host.apk to prebuilt/ before the build.
# PRESIGNED keeps its own (debug) signature, so `tools/deploy.sh --host` can still update it in
# place with `adb install -r`. Overriding a package removes it from the image: every launcher, and
# the stock apps r1os doesn't ship (ADR-0013's keep-list). Make ignores names that don't exist.
include $(CLEAR_VARS)
LOCAL_MODULE := R1osHost
LOCAL_MODULE_CLASS := APPS
LOCAL_MODULE_SUFFIX := .apk
LOCAL_SRC_FILES := prebuilt/r1os-host.apk
LOCAL_CERTIFICATE := PRESIGNED
LOCAL_PRIVILEGED_MODULE := true
LOCAL_DEX_PREOPT := false
LOCAL_ENFORCE_USES_LIBRARIES := false
LOCAL_OVERRIDES_PACKAGES := \
    Launcher3 Launcher3QuickStep Launcher3Go Launcher3QuickStepGo Trebuchet TrebuchetQuickStep \
    Calendar Etar Camera2 Aperture Snap DeskClock Gallery2 Glimpse Music Eleven MusicFX \
    QuickSearchBox Browser2 Jelly Email Messaging messaging Dialer Contacts ExactCalculator Calculator \
    LiveWallpapersPicker WallpaperPicker WallpaperPicker2 ThemePicker SoundRecorder Recorder \
    PhotoTable BasicDreams OneTimeInitializer PartnerBookmarksProvider Stk Traceur EasterEgg \
    $(R1OS_CUT_1)
include $(BUILD_PREBUILT)

# r1d (crates/r1d) as an init service. The script runs whatever tools/deploy.sh last put in
# /data/r1os/bin, the way r1os-base's service.sh did.
include $(CLEAR_VARS)
LOCAL_MODULE := r1os-r1d
LOCAL_MODULE_CLASS := EXECUTABLES
LOCAL_SRC_FILES := bin/r1os-r1d
LOCAL_CHECK_ELF_FILES := false
include $(BUILD_PREBUILT)

# Verbose boot (crates/r1-bootlog, T-101): tools/build-bootlog.sh builds it into out/bootlog, and
# tools/build-image.sh copies it to prebuilt/ before the build.
include $(CLEAR_VARS)
LOCAL_MODULE := r1os-bootlog
LOCAL_MODULE_CLASS := EXECUTABLES
LOCAL_SRC_FILES := prebuilt/r1os-bootlog
LOCAL_CHECK_ELF_FILES := false
include $(BUILD_PREBUILT)

include $(CLEAR_VARS)
LOCAL_MODULE := r1os.rc
LOCAL_MODULE_CLASS := ETC
LOCAL_SRC_FILES := init/r1os.rc
LOCAL_MODULE_RELATIVE_PATH := init
include $(BUILD_PREBUILT)

# android.software.app_widgets: Rabbit's vendor doesn't declare it (research, R0).
include $(CLEAR_VARS)
LOCAL_MODULE := r1os-features.xml
LOCAL_MODULE_CLASS := ETC
LOCAL_SRC_FILES := permissions/r1os-features.xml
LOCAL_MODULE_RELATIVE_PATH := permissions
include $(BUILD_PREBUILT)

# The host app may turn off the status bar and notification shade (STATUS_BAR, privileged).
include $(CLEAR_VARS)
LOCAL_MODULE := privapp-permissions-r1os.xml
LOCAL_MODULE_CLASS := ETC
LOCAL_SRC_FILES := permissions/privapp-permissions-r1os.xml
LOCAL_MODULE_RELATIVE_PATH := permissions
include $(BUILD_PREBUILT)

# Aurora Store (ADR-0013, Q23): Google Play's catalog without a Google account, the only Android app the image
# ships. Its `preload` build, from tools/fetch-aurora.sh (pinned, signature checked), as a privileged app: it
# holds INSTALL_PACKAGES, so an install the user taps needs no second confirmation. Presigned by Aurora OSS, so
# its own updates install over it.
# In priv-app by path, with LOCAL_PRIVILEGED_MODULE false: a privileged module's dex gets uncompressed, which
# rewrites the APK and breaks Aurora's v2 signature (v0.2.11's first build: "DOES NOT VERIFY"), and base_rules
# marks anything under priv-app privileged unless the module says otherwise. Android decides privilege by the
# directory. build-image.sh's export checks the APK went in unchanged.
include $(CLEAR_VARS)
LOCAL_MODULE := AuroraStore
LOCAL_MODULE_CLASS := APPS
LOCAL_MODULE_SUFFIX := .apk
LOCAL_SRC_FILES := prebuilt/AuroraStore.apk
LOCAL_CERTIFICATE := PRESIGNED
LOCAL_MODULE_PATH := $(TARGET_OUT)/priv-app
LOCAL_PRIVILEGED_MODULE := false
LOCAL_DEX_PREOPT := false
LOCAL_ENFORCE_USES_LIBRARIES := false
include $(BUILD_PREBUILT)

# The host may call IAdbManager, a hidden interface, to answer adb's confirmations (R4's keep-list).
include $(CLEAR_VARS)
LOCAL_MODULE := hiddenapi-r1os.xml
LOCAL_MODULE_CLASS := ETC
LOCAL_SRC_FILES := permissions/hiddenapi-r1os.xml
LOCAL_MODULE_RELATIVE_PATH := sysconfig
include $(BUILD_PREBUILT)

# The host app is a system app with no one to tap "Allow": grant it the microphone up front.
include $(CLEAR_VARS)
LOCAL_MODULE := default-permissions-r1os.xml
LOCAL_MODULE_CLASS := ETC
LOCAL_SRC_FILES := permissions/default-permissions-r1os.xml
LOCAL_MODULE_RELATIVE_PATH := default-permissions
include $(BUILD_PREBUILT)

# The side button (mtk-kpd 116) as BUTTON_1 WAKE instead of POWER (r1os-base, R1).
include $(CLEAR_VARS)
LOCAL_MODULE := mtk-kpd.kl
LOCAL_MODULE_CLASS := ETC
LOCAL_SRC_FILES := keylayout/mtk-kpd.kl
LOCAL_MODULE_PATH := $(TARGET_OUT)/usr/keylayout
include $(BUILD_PREBUILT)
