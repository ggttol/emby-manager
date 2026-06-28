import { ReactNode } from 'react';

export function VirtualList<T>({
  items,
  render
}: {
  items: T[];
  render: (item: T, index: number) => ReactNode;
}) {
  return <div className="virtualList">{items.map(render)}</div>;
}

export function Combobox({
  value,
  options,
  onChange
}: {
  value: string;
  options: string[];
  onChange: (value: string) => void;
}) {
  return (
    <select className="input" value={value} onChange={(e) => onChange(e.target.value)}>
      {options.map((option) => <option key={option} value={option}>{option}</option>)}
    </select>
  );
}

export function DataTable({
  columns,
  rows
}: {
  columns: string[];
  rows: Array<Array<ReactNode>>;
}) {
  return (
    <table className="dataTable">
      <thead><tr>{columns.map((col) => <th key={col}>{col}</th>)}</tr></thead>
      <tbody>{rows.map((row, i) => <tr key={i}>{row.map((cell, j) => <td key={j}>{cell}</td>)}</tr>)}</tbody>
    </table>
  );
}

