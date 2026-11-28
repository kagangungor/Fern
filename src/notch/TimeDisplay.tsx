import { memo, useEffect, useState } from "react";
import { formatLocalClock, msUntilNextMinute } from "./utils";

export const TimeDisplay = memo(function TimeDisplay({ hour12 }: { hour12: boolean }) {
  const [text, setText] = useState(() => formatLocalClock(new Date(), hour12));

  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | null = null;
    const sync = () => {
      const now = new Date();
      setText(formatLocalClock(now, hour12));
      timer = setTimeout(sync, msUntilNextMinute(now));
    };
    sync();
    return () => {
      if (timer) clearTimeout(timer);
    };
  }, [hour12]);

  return <>{text}</>;
});
