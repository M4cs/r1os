// The site's pages: home, the docs (a page per markdown file in docs/content, loaded when you first open them), and a
// lost page.

import { createBrowserRouter } from "react-router";
import { Root } from "./site/Root";
import { Home } from "./home/Home";
import { Lost } from "./site/Lost";

const docs = () => import("./docs/Docs");

export const router = createBrowserRouter([
  {
    element: <Root />,
    errorElement: <Lost />,
    children: [
      { index: true, element: <Home /> },
      {
        path: "docs",
        lazy: async () => ({ Component: (await docs()).DocsLayout }),
        children: [
          { index: true, lazy: async () => ({ Component: (await docs()).DocPage }) },
          { path: ":slug", lazy: async () => ({ Component: (await docs()).DocPage }) },
        ],
      },
      { path: "*", element: <Lost /> },
    ],
  },
]);
