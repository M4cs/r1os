// On a real R1: the feature reel (public/media, T-200), screen recordings from the device laid over the drawn R1. The
// page's other screens are drawn, so this says what it is and how it was cut. It loads and plays only while it's on
// screen, and rests when it's scrolled away. With reduced motion it's the poster and the controls.

import { motion, useReducedMotion } from "motion/react";
import { useEffect, useRef } from "react";
import { rise } from "../site/motion";
import { useSeen } from "../site/seen";
import { Head } from "./parts";

export function RealOne() {
  const box = useRef<HTMLDivElement>(null);
  const video = useRef<HTMLVideoElement>(null);
  const seen = useSeen(box, 0.25);
  const reduce = useReducedMotion();
  useEffect(() => {
    const v = video.current;
    if (!v || reduce) return;
    if (seen) void v.play().catch(() => {});
    else v.pause();
  }, [seen, reduce]);
  return (
    <section className="section" id="real">
      <div className="wrap">
        <Head layout="quiet" title="on a real r1." sub="the screens on this page are drawn. these are recorded on the device, then cut and sped up." link={{ to: "/docs/using", label: "using r1os" }} />
        <motion.div className="real" ref={box} {...(reduce ? {} : rise)}>
          <video
            ref={video}
            muted
            loop
            playsInline
            preload="none"
            controls={!!reduce}
            poster="/media/r1os-feature-poster.png"
            aria-label="r1os on an r1: the glance screen, turning through agents, designing one, a spoken question answered with a card, an app on the wheel and the built-in tools."
          >
            <source src="/media/r1os-feature.webm" type="video/webm" />
            <source src="/media/r1os-feature.mp4" type="video/mp4" />
          </video>
        </motion.div>
      </div>
    </section>
  );
}
