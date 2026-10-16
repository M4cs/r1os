# r1os's product layer on TrebleDroid's AOSP 14 GSI (ADR-0013, roadmap R4). First version,
# 2026-10-09. TrebleDroid's generate.sh inherits this into every treble_* target; we build
# treble_arm64_bvN (arm64, A/B, vanilla, no su).
#
# What this version does:
#   - the host app as a privileged system app and the only home screen (it overrides every
#     launcher, and the stock apps r1os doesn't ship);
#   - r1d as an init service in its own (for now permissive) SELinux domain, running the binary
#     tools/deploy.sh puts in /data/r1os/bin, so the dev loop is unchanged;
#   - what r1os-base did through Magisk: the side button as BUTTON_1, the app-widgets feature,
#     and Rabbit's USB CUA mode off.
# Since: Aurora Store, the boot animation (your agents), verbose boot, and boot 36 s sooner (v0.2.15-17).
# Not yet: SystemUI and Settings removed (r1d's loopback gap, which held it back, is closed by its
# per-boot token: ADR-0026), r1d and the speech libraries baked into the image, our branding.

PRODUCT_PACKAGES += \
    R1osHost \
    AuroraStore \
    r1os-r1d \
    r1os-bootlog \
    r1os.rc \
    r1os-features.xml \
    default-permissions-r1os.xml \
    privapp-permissions-r1os.xml \
    hiddenapi-r1os.xml \
    mtk-kpd.kl

SYSTEM_EXT_PRIVATE_SEPOLICY_DIRS += vendor/r1os/sepolicy

# The boot animation (T-029): r1os's own cast, drawn by the host (tools/r1h bootanim default) into out/bootanim,
# which tools/build-image.sh copies to prebuilt/ (without it, a still of it: device/aosp/bootanimation-still.png).
# Your agents' (r1d's, /data/r1os/boot) is bound over it at boot.
PRODUCT_COPY_FILES += vendor/r1os/prebuilt/bootanimation.zip:$(TARGET_COPY_OUT_PRODUCT)/media/bootanimation.zip

# Framework settings r1os overrides (overlay/): the host answers adb's confirmations (roadmap R4's keep-list).
PRODUCT_PACKAGE_OVERLAYS += vendor/r1os/overlay

PRODUCT_SYSTEM_PROPERTIES += \
    ro.r1os.image=1 \
    ro.r1os.image.version=0.2.18

# r1os's own keys sign every APK the build signs (T-133, ADR-0062), not AOSP's public test-keys:
# platform, shared, media, networkstack, nfc, bluetooth and sdk_sandbox come from the same folder.
# tools/build-image.sh copies them from ~/.r1os/keys ($KEYS, tools/make-keys.sh); never committed.
# The host is signed with the same platform key, so it holds its signature permissions.
PRODUCT_DEFAULT_DEV_CERTIFICATE := vendor/r1os-keys/releasekey

# Product properties load after the vendor's, so these win over Rabbit's /vendor/build.prop:
#   - the R1's 480x640 panel looks "really zoomed in" at the GSI's 320;
#   - Rabbit's vendor sets ro.vendor.cua.enable=1 (its USB "CUA" mode: ADB + keyboard + mouse +
#     WebUSB, PID 0x2303). init can't change it later because ro. props are set once (seen on
#     v0.2.0, 2026-10-09). r1os-base did this with Magisk's resetprop;
#   - no Android keyguard: the host's glance screen is the R1's lock screen (ADR-0019).
PRODUCT_PRODUCT_PROPERTIES += \
    ro.sf.lcd_density=180 \
    ro.vendor.cua.enable=0 \
    ro.lockscreen.disable.default=true
