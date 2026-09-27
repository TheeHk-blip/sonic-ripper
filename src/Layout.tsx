import { ReactNode } from 'react';
import LogPanel from './components/LogPanel';
import TitleBar from './components/Titlebar';

function Layout({ children }: { children: ReactNode }) {
  return (
    <div className="flex flex-col h-screen w-full">
      <TitleBar />
      <div className="mb-15" />
      <div className="flex flex-col md:flex-row overflow-auto">
        <main className="flex-1 overflow-y-auto">{children}</main>
        <LogPanel />
      </div>
    </div>
  );
}

export default Layout;
