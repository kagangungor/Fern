import { motion } from "framer-motion";

export function UpdateActivity() {
  return (
    <div className="update-activity">
      <motion.div
        className="update-activity-glow"
        animate={{ opacity: [0.5, 1, 0.5], scale: [0.9, 1, 0.9] }}
        transition={{ duration: 1.6, repeat: Infinity, ease: "easeInOut" }}
      >
        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="#32D74B" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round">
          <path d="M12 5v11" />
          <path d="M6 12l6 6 6-6" />
        </svg>
      </motion.div>
    </div>
  );
}
