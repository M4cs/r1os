import { createRoot } from "react-dom/client";
import { App } from "./App";
import { applyLook } from "./theme";
import "./styles.css";

applyLook(null);
createRoot(document.getElementById("root")!).render(<App />);
