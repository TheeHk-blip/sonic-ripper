import React from 'react';
import { motion } from 'motion/react';
import { Link2, Sliders, Download, Check } from 'lucide-react';
import { useI18n } from '../lib/i18n';

export type FlowStep = 'source' | 'configure' | 'results';

interface StepProgressProps {
  current: FlowStep;
}

const themeColor = (name: string) => {
  if (typeof window === 'undefined') return;
  const value = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return value;
};

const BRAND = themeColor('--color-charcoal');
const BRAND_ACTIVE_BG = themeColor('--color-olive');
const BORDER_IDLE = themeColor('--color-cream');

export default function StepProgress({ current }: StepProgressProps) {
  const { t } = useI18n();

  const steps: { key: FlowStep; label: string; icon: React.ReactNode }[] = [
    { key: 'source', label: t.stepSource, icon: <Link2 className="size-5" /> },
    { key: 'configure', label: t.stepConfigure, icon: <Sliders className="size-5" /> },
    { key: 'results', label: t.stepDownload, icon: <Download className="size-5" /> },
  ];

  const currentIndex = steps.findIndex(s => s.key === current);

  return (
    <div className="flex items-center w-full bg-olive/35 rounded-2xl px-3 py-2 mb-2 mt-5">
      {steps.map((s, idx) => {
        const isDone = idx < currentIndex;
        const isActive = idx === currentIndex;

        return (
          <React.Fragment key={s.key}>
            <div className="flex items-center gap-3 shrink-0" id={`step-node-${s.key}`}>
              <div className="relative w-9 h-9 flex items-center justify-center shrink-0">
                <motion.div
                  className="absolute inset-0 rounded-sm bg-charcoal"
                  animate={{
                    borderColor: isDone || isActive ? BRAND : BORDER_IDLE,
                    backgroundColor: isDone ? BRAND : isActive ? BRAND_ACTIVE_BG : 'transparent',
                  }}
                  transition={{ duration: 0.35, ease: 'easeOut' }}
                />
                <span
                  className={`relative z-10 ${isDone ? 'text-gold' : isActive ? 'text-cream' : 'text-rust'}`}
                >
                  {isDone ? <Check className="size-5" /> : s.icon}
                </span>
              </div>
              <div className="hidden sm:block">
                <p
                  className={`font-thin transition-colors duration-300 ${
                    isActive ? 'text-olive text-lg' : isDone ? 'text-rust' : 'text-cream'
                  }`}
                >
                  {s.label}
                </p>
              </div>
            </div>

            {idx < steps.length - 1 && (
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
