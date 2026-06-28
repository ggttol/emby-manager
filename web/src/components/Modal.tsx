import { ReactNode } from 'react';

export function Modal({
  title,
  children,
  onClose
}: {
  title: string;
  children: ReactNode;
  onClose: () => void;
}) {
  return (
    <div className="overlay" onMouseDown={onClose}>
      <section className="modal" onMouseDown={(e) => e.stopPropagation()}>
        <header className="modalHead">
          <h2>{title}</h2>
          <button className="iconBtn" onClick={onClose} aria-label="关闭">×</button>
        </header>
        {children}
      </section>
    </div>
  );
}

export function ConfirmDanger({
  title,
  body,
  confirmText,
  onCancel,
  onConfirm
}: {
  title: string;
  body: ReactNode;
  confirmText: string;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  return (
    <Modal title={title} onClose={onCancel}>
      <div className="modalBody">{body}</div>
      <footer className="modalActions">
        <button className="btn ghost" onClick={onCancel}>取消</button>
        <button className="btn danger" onClick={onConfirm}>{confirmText}</button>
      </footer>
    </Modal>
  );
}

