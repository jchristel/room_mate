// What the page says while there is nothing on it yet.
//
// It names the live page and links to it, rather than showing an empty plan:
// during the port `/viewer/` is reachable by anyone who guesses the URL, and a
// blank viewer with no explanation reads as a broken one. Deleted by the
// cutover slice (B10), together with `static/index.html`.

export function PortNotice() {
  return (
    <div className="port-notice">
      <h2>The viewer is being ported</h2>
      <p>
        This page is the React rebuild of the room plan, built one slice at a
        time. Nothing is drawn here yet.
      </p>
      <p>
        The working viewer is at <a href="/">/</a>.
      </p>
    </div>
  );
}
