// What an agent is doing, in words, while a tool runs: the simulator's chat and the companion's chats show these
// (the R1's own list is ChatScreen.TOOL_LABEL in the host, with the same toolLabel). Tools not here go through toolLabel.

export const TOOL_LABEL: Record<string, string> = {
  create_app: "Starting a build",
  edit_app: "Changing an app",
  list_apps: "Looking at your apps",
  describe_app: "Looking at an app",
  query_app: "Checking your data",
  run_action: "Updating an app",
  undo_action: "Undoing that",
  open_app: "Opening an app",
  app_history: "Looking at the history",
  undo_app_change: "Undoing the change",
  delete_app: "Removing an app",
  restore_app: "Checking the trash",
  stop_build: "Stopping the build",
  glance_widgets: "Arranging your widgets",
  list_android_apps: "Looking at your Android apps",
  open_android_app: "Opening an Android app",
  show_android_app: "Opening the app store",
  show_card: "Making a card",
  list_notifications: "Checking your notifications",
  open_notification: "Opening it",
  dismiss_notification: "Clearing it",
  reply_notification: "Sending your reply",
  control_media: "Checking the player",
  remember: "Remembering that",
  list_memories: "Checking what I remember",
  forget: "Forgetting that",
  list_agents: "Checking the others",
  ask_agent: "Asking another agent",
  tell_agent: "Leaving a note",
  read_feed: "Reading the news",
  set_alarm: "Setting an alarm",
  set_reminder: "Setting a reminder",
  set_timer: "Setting a timer",
  list_alarms: "Checking your alarms",
  cancel_alarm: "Cancelling it",
  stop_alarm: "Stopping it",
  set_time_zone: "Setting the time zone",
  list_media: "Looking at your photos",
  view_photo: "Looking at a photo",
  label_media: "Labelling it",
  start_recording: "Starting the recorder",
  stop_recording: "Stopping the recorder",
  read_recording: "Reading the recording",
  journal_add: "Writing in your journal",
  journal_search: "Searching your journal",
  journal_list: "Reading your journal",
  device_settings: "Checking the R1's settings",
  change_device_settings: "Changing the R1's settings",
  set_app_permission: "Asking about a permission",
  set_look: "Changing the look",
  set_agent_look: "Changing my look",
  WebSearch: "Searching the web",
  WebFetch: "Reading a page",
  Bash: "Working it out",
  Read: "Reading a file",
  Write: "Writing a file",
  Edit: "Editing a file",
  Glob: "Looking through files",
  Grep: "Searching files",
};

/** A connection's server as people know it: "claude_ai_Google_Calendar" → "Google Calendar"; Codex's apps are ChatGPT's. */
function service(server: string): string {
  if (server === "codex_apps") return "ChatGPT";
  return server.replace(/^claude_ai_/, "").replace(/[_-]+/g, " ").trim();
}

/** A tool's own name in words: "search_threads" → "search threads", "getEvents" → "get events". */
function words(name: string): string {
  return name.replace(/([a-z0-9])([A-Z])/g, "$1 $2").replace(/[_-]+/g, " ").trim().toLowerCase();
}

/**
 * What a badge says for a tool: r1os's own in words (TOOL_LABEL); a connection's as its service and what it does
 * ("mcp__claude_ai_Gmail__search_threads" → "Gmail · search threads"); any other in words.
 */
export function toolLabel(name: string): string {
  const known = TOOL_LABEL[name];
  if (known) return known;
  const mcp = /^mcp__(.+?)__(.+)$/.exec(name);
  if (mcp) return `${service(mcp[1])} · ${words(mcp[2])}`;
  const w = words(name);
  return w.charAt(0).toUpperCase() + w.slice(1);
}
