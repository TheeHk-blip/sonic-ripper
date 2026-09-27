import React from 'react';
import { motion } from 'motion/react';
import { Link2, Sliders, Download, Check } from 'lucide-react';

export type FlowStep = 'source' | 'configure' | 'results';

interface StepProgressProps {
  current: FlowStep;
}

const STEPS: { key: FlowStep; label: string; icon: React.ReactNode }[] = [
  { key: 'source', label: 'Add Source', icon: <Link2 className="size-5" /> },
  { key: 'configure', label: 'Configure', icon: <Sliders className="size-5" /> },
  { key: 'results', label: 'Download', icon: <Download className="size-5" /> },
];

export default function StepProgress({ current }: StepProgressProps) {
  const currentIndex = STEPS.findIndex(s => s.key === current);

  return (
    <div className="flex items-center w-full bg-olive/40 rounded-sm px-3 py-2">
      {STEPS.map((s, idx) => {
        const isDone = idx < currentIndex;
        const isActive = idx === currentIndex;

        return (
          <React.Fragment key={s.key}>
            <div className="flex items-center gap-3 shrink-0" id={`step-node-${s.key}`}>
              <div className="relative w-9 h-9 flex items-center justify-center shrink-0">
                <div
                  className={`absolute inset-0 rounded-sm transition-colors duration-300 ease-out ${
                    isDone ? 'bg-charcoal' : isActive ? 'bg-olive' : 'bg-transparent'
                  }`}
                />
                <span
                  className={`relative z-10 ${isDone ? 'text-gold' : isActive ? 'text-cream' : 'text-rust'}`}
                >
                  {isDone ? <Check className="size-5" /> : s.icon}
                </span>
              </div>
              <div className="hidden sm:block">
                <motion.p
                  animate={{ scale: isActive ? 1.08 : 1 }}
                  transition={{ duration: 0.35, ease: 'easeOut' }}
                  style={{ transformOrigin: 'left center' }}
                  className={`font-thin transition-colors duration-300 ${
                    isActive ? 'text-gold' : isDone ? 'text-rust' : 'text-cream'
                  }`}
                >
                  {s.label}
                </motion.p>
              </div>
            </div>

            {idx < STEPS.length - 1 && (
              <div className="flex-1 h-px mx-3 sm:mx-4 relative overflow-hidden">
                <motion.div
                  className="absolute inset-y-0 left-0 bg-olive"
                  initial={false}
                  animate={{ width: idx < currentIndex ? '100%' : '0%' }}
                  transition={{ duration: 0.45, ease: 'easeOut' }}
                />
              </div>
            )}
          </React.Fragment>
        );
      })}
    </div>
  );
}
