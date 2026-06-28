import { createContext, ReactNode, useContext, useMemo, useState } from 'react';

type Toast = { id: number; text: string; tone?: 'ok' | 'warn' | 'error' | 'info' };
type ToastContextValue = { push: (text: string, tone?: Toast['tone']) => void };

const ToastContext = createContext<ToastContextValue | null>(null);

export function ToastProvider({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<Toast[]>([]);
  const value = useMemo<ToastContextValue>(() => ({
    push(text, tone = 'info') {
      const id = Date.now() + Math.random();
      setItems((prev) => [...prev, { id, text, tone }]);
      window.setTimeout(() => setItems((prev) => prev.filter((x) => x.id !== id)), 4200);
    }
  }), []);
  return (
    <ToastContext.Provider value={value}>
      {children}
      <div className="toastStack">
        {items.map((item) => (
          <div className={`toast ${item.tone || 'info'}`} key={item.id}>{item.text}</div>
        ))}
      </div>
    </ToastContext.Provider>
  );
}

export function useToast() {
  const value = useContext(ToastContext);
  if (!value) throw new Error('useToast must be used inside ToastProvider');
  return value;
}

