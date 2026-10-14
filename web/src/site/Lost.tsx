// A page that isn't there: an agent, asleep.

import { Link, isRouteErrorResponse, useRouteError } from "react-router";
import { Character } from "@shell/ui/Character";

export function Lost() {
  const error = useRouteError();
  const broke = error && !(isRouteErrorResponse(error) && error.status === 404);
  return (
    <section className="lost">
      <div>
        <div style={{ display: "grid", placeItems: "center" }}>
          <Character form="ghost" color="indigo" finish="grain" state={broke ? "error" : "sleeping"} size={150} />
        </div>
        <h1>{broke ? "something went wrong" : "nothing here"}</h1>
        <p className="muted">{broke ? "this page tripped over itself. try again, or head home." : "this page isn't on the wheel. it may have moved."}</p>
        <Link to="/" className="btn primary">
          back home
        </Link>
      </div>
    </section>
  );
}
