// Pictures for the install's steps done with your hands (T-171): the R1 from the front, as the look page draws it
// (an orange body, the screen under black glass at its left, the camera's window and the wheel at its right), with
// the side button on its right edge and the USB-C port on its left. Pressing the side button for the unlock; plugging
// a powered-off R1 in for the preloader. Each moves gently, and keeps still for those who'd rather it didn't.

/** The R1, front on, its body at (70, 25) and 140 across, its screen showing [screen]. */
function R1({ screen, id }: { screen: React.ReactNode; id: string }) {
  return (
    <>
      <defs>
        <linearGradient id={`${id}-body`} x1="0" y1="0" x2="0.3" y2="1">
          <stop offset="0" stopColor="#ff6a24" />
          <stop offset="0.4" stopColor="#ff4f0a" />
          <stop offset="1" stopColor="#f54700" />
        </linearGradient>
        <linearGradient id={`${id}-roller`} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#7a1600" />
          <stop offset="0.35" stopColor="#ff6a34" />
          <stop offset="0.7" stopColor="#d93200" />
          <stop offset="1" stopColor="#6e1200" />
        </linearGradient>
      </defs>
      {/* The side button (right edge, near the top) and the wheel's edge (beside its slot). */}
      <rect className="hands-button" x="206" y="38" width="8.5" height="18" rx="3" fill="#9c2600" />
      <rect x="208" y="73" width="4" height="16" rx="1.5" fill="#c9c9c9" />
      {/* The USB-C port, on the left edge. */}
      <rect x="67" y="88" width="6" height="16" rx="2.5" fill="#5e1200" />
      <rect x="70" y="25" width="140" height="140" rx="12" fill={`url(#${id}-body)`} />
      <rect x="75" y="30" width="99" height="130" rx="8" fill="#060606" />
      <rect x="80" y="36" width="89" height="118" rx="3" fill="#0e0e10" />
      {screen}
      <rect x="179" y="31" width="27" height="25" rx="4.5" fill="#b82200" />
      <rect x="182" y="34" width="21" height="19" rx="3" fill="#ff4a1a" />
      <circle cx="192.5" cy="43.5" r="4.2" fill="#07080a" stroke="#ffffffaa" strokeWidth="1" />
      <rect x="179" y="66" width="27" height="27" rx="4" fill="#5e1200" />
      <rect x="181" y="69" width="25" height="21" rx="3" fill={`url(#${id}-roller)`} />
    </>
  );
}

/** Pressing the side button: the R1's bootloader asking whether to unlock. */
export function PressSideButton() {
  return (
    <svg className="hands-svg" viewBox="56 14 196 162" role="img" aria-label="press the side button on the r1's right edge">
      <R1
        id="press"
        screen={
          <g fontFamily="ui-monospace, Consolas, monospace" fontSize="6.5" fill="#d8d8d8">
            <text x="85" y="62">unlock the</text>
            <text x="85" y="71">bootloader?</text>
            <text x="85" y="92" fill="#8a8f98">press the side</text>
            <text x="85" y="101" fill="#8a8f98">button to say yes</text>
          </g>
        }
      />
      {/* Rings where it's pressed, and an arrow pressing in. */}
      <circle className="hands-ring" cx="214" cy="47" r="9" fill="none" style={{ stroke: "var(--accent)" }} strokeWidth="2" />
      <g className="hands-push">
        <path d="M222 47 h22" style={{ stroke: "var(--accent)" }} strokeWidth="4" strokeLinecap="round" />
        <path d="M229 39 l-8 8 l8 8" fill="none" style={{ stroke: "var(--accent)" }} strokeWidth="4" strokeLinecap="round" strokeLinejoin="round" />
      </g>
    </svg>
  );
}

/** A powered-off R1 (its screen dark), and the cable going into its port. */
export function PlugInOff() {
  return (
    <svg className="hands-svg" viewBox="-2 14 222 162" role="img" aria-label="with the r1 off, plug the cable into its left edge">
      <R1
        id="plug"
        screen={
          <g opacity="0.55">
            <circle cx="124.5" cy="91" r="11" fill="none" stroke="#5a5d64" strokeWidth="2.5" />
            <path d="M124.5 77 v12" stroke="#5a5d64" strokeWidth="2.5" strokeLinecap="round" />
          </g>
        }
      />
      <g className="hands-plug">
        <path d="M-4 128 C 18 128, 22 96, 40 96" fill="none" stroke="#9aa0a8" strokeWidth="4" strokeLinecap="round" />
        <rect x="38" y="89" width="20" height="14" rx="3" fill="#d6d8dc" />
        <rect x="57" y="92" width="9" height="8" rx="2" fill="#9aa0a8" />
      </g>
      <path className="hands-hint" d="M44 76 h16 m-6 -6 l6 6 l-6 6" fill="none" style={{ stroke: "var(--accent)" }} strokeWidth="3" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}
