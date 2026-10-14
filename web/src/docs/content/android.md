---
title: Android apps
description: Android 15 apps, and agents that use them.
group: the os
order: 2.5
---

# Android apps

Under r1os is a full Android 15 device. Install apps, use them with the wheel and your voice, and let your agents use them while you watch.

## A full Android device

- **Android 15:** r1os's own image, built from AOSP, with r1os as the home screen.
- **Aurora Store** is built in: Google Play's catalog, with no Google account. Agents can open it on an app's page; the install tap is yours.
- **No Google services.** Apps that need Google Play Services or Google's integrity checks may not work. microG, offered when an app needs it, is planned.

## Your apps in r1os

- **The launcher** lists them under Android apps, with **Get apps** for Aurora. A long press pins one to home.
- **Notifications** show on the glance screen. Agents can read, open and dismiss them, and reply when you ask.
- **The wheel** moves like a d-pad and a click selects. **Hold** talks to your agent over any app; **double click** goes back to r1os.

Apps left in the background for 10 minutes close, unless they're playing.

## Agents in your apps

The agent tries what needs no screen first: a connected app for the service, then an app link (a page in the app, or the media controls), then the app window.

### The app window

- **A screen of its own:** a hidden second display, 600 × 1200 at 15 fps. The wheel and button stay yours.
- **Your apps, signed in as you.** No cloud phone, and r1os stores no passwords.
- **Live in the chat,** with the agent's pointer, and on the glance screen. Touch it for the full window, where your touches reach the app and two clicks stop the agent.
- **What the agent sees:** each step, the screen's text and a picture, sent to its model provider like the rest of the chat.
- **Take over any time.** When the agent needs you, the window comes up with **done** and **not now**; **let me use it** moves the app to your screen.
- **Screen on only.** The task waits while the screen is off; working with it off is planned.

### Approvals

- **An app's first use:** a card, once per app. Apps whose terms forbid automated use say so.
- **The first tap or typing in a task:** a card, by the agent's `android` [level](/docs/agents#approvals). Looking runs.
- **A final step** (an order, checkout, payment, ride, subscription or message): a card every time, whatever the level, with the order as the app's screen shows it: items, total, where it goes and how it's paid. The agent presses only after you allow it.

### Passwords and payments

- **Secure screens stay black.** Card and password screens, and some carts, come out black to the agent, and it hands the app to you.
- **Password fields refuse its typing.** Passwords, card numbers and codes are yours.
- **Permission prompts are yours.** While Android asks, the agent can't tap or type.

## Measured

On r1os's own suite of 21 tasks (8 on the web, 3 in Spotify, 10 food orders in Uber Eats), one Claude agent on a plugged-in device passed all 21 in each of its last two full runs. Orders stopped at the add-to-cart button. A task takes 13 s to 11 minutes, but each step is quick:

| Step | Before | After |
|---|---|---|
| Read a screen | 0.86 s | 0.52 s |
| Scroll (median) | 4.96 s | 0.95 s |
| Open a running app | 9.3 s | 2.0 s |
| Check a tap by place, screen unchanged | 2–6.6 s | 155 ms |

Your own screen stays smooth while it works ([more numbers](/docs/architecture#speed-and-battery)). Battery use with the window open isn't measured yet.

## Your account

What an agent does in an app, it does on your account. r1os isn't affiliated with any app named here.
