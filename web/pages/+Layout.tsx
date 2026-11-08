import "./tailwind.css";

export default function Layout({ children }: { children: React.ReactNode }) {
  return <main className="min-h-screen bg-base-100 text-base-content">{children}</main>;
}
