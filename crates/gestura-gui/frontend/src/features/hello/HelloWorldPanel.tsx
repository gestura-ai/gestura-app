import './HelloWorldPanel.css';

export default function HelloWorldPanel() {
  return (
    <section className="hello-world-panel" aria-labelledby="hello-world-title">
      <div className="hello-world-card">
        <p className="hello-world-eyebrow">Tauri demo</p>
        <h2 id="hello-world-title">Hello, World!</h2>
        <p className="hello-world-copy">
          This is a small Gestura Tauri GUI screen rendered with React.
        </p>
      </div>
    </section>
  );
}