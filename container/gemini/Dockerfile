FROM rclaw-base:latest

# Install Gemini CLI globally
RUN npm install -g @google/gemini-cli

USER rclaw

# Copy the entrypoint script
COPY --chown=rclaw:rclaw ../entrypoint.js /home/rclaw/entrypoint.js

# Default entrypoint
HEALTHCHECK --interval=1s --timeout=1s --start-period=1s --retries=3 \
  CMD node -e "if (require('fs').existsSync('/home/rclaw/entrypoint.js')) process.exit(0); else process.exit(1);"

ENTRYPOINT ["tail", "-f", "/dev/null"]
