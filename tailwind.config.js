/** @type {import('tailwindcss').Config} */
module.exports = {
  content: [
    // Scan all HTML templates
    "./web/templates/**/*.html",
    // Scan Go files for class strings (if any)
    "./internal/**/*.go",
  ],
  theme: {
    extend: {
      // TODO: Add custom theme extensions here
      // colors: {
      //   'brand': '#your-color',
      // },
      // fontFamily: {
      //   'sans': ['Your Font', 'sans-serif'],
      // },
    },
  },
  plugins: [
    // TODO: Add Tailwind plugins if needed
    // require('@tailwindcss/forms'),
    // require('@tailwindcss/typography'),
  ],
}
