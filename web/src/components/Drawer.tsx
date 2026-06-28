import { ReactNode } from 'react';
import { X } from 'lucide-react';

export function Drawer({
  title,
  children,
  onClose
}: {
  title: string;
  children: ReactNode;
  onClose: () => void;
}) {
  return (
    <div className="drawerOverlay" onMouseDown={onClose}>
      <aside className="drawer" onMouseDown={(e) => e.stopPropagation()}>
        <header className="drawerHead">
          <h2>{title}</h2>
          <button className="iconBtn" onClick={onClose} aria-label="关闭"><X size={18} /></button>
        </header>
        {children}
      </aside>
    </div>
  );
}

