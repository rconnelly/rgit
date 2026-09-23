(function () {
  document.querySelectorAll("[data-copy-url]").forEach(function (root) {
    var field = root.querySelector("[data-copy-url-field]");
    var button = root.querySelector("[data-copy-url-button]");
    var status = root.querySelector("[data-copy-url-status]")
      || (root.parentElement && root.parentElement.querySelector("[data-copy-url-status]"));
    var idleLabel = button ? button.textContent : "Copy";

    function value() {
      if (field && field.value) return field.value;
      return root.getAttribute("data-copy-url") || "";
    }

    function selectField() {
      if (!field || typeof field.select !== "function") return;
      field.focus();
      field.select();
    }

    function setCopied(ok) {
      if (button) button.textContent = ok ? "Copied" : idleLabel;
      if (status) status.textContent = ok ? "Copied clone URL." : "";
      window.setTimeout(function () {
        if (button) button.textContent = idleLabel;
        if (status) status.textContent = "";
      }, 1600);
    }

    if (field) {
      field.addEventListener("focus", selectField);
      field.addEventListener("click", selectField);
    }

    if (!button) return;

    button.addEventListener("click", function () {
      var text = value();
      if (!text) return;
      if (navigator.clipboard && typeof navigator.clipboard.writeText === "function") {
        navigator.clipboard.writeText(text).then(function () {
          setCopied(true);
        }).catch(function () {
          selectField();
        });
        return;
      }
      selectField();
    });
  });
})();
