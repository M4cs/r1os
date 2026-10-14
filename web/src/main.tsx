import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { RouterProvider } from "react-router";
import { router } from "./router";
import { LookProvider } from "./site/look";
import "./styles/base.css";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <LookProvider>
      <RouterProvider router={router} />
    </LookProvider>
  </StrictMode>,
);
