const gulp = require('gulp');
const sass = require('gulp-sass');
const jsImport = require('gulp-js-import');
const cssimport = require('gulp-cssimport');

gulp.task('sass', () => {
  return gulp.src('./sass/**/*.scss')
    .pipe(cssimport({
      extensions: ["css"]
    }))
    .pipe(sass({
      includePaths: [
//        'node_modules/govuk_template_mustache/assets/stylesheets', // 1
        'node_modules/accessible-autocomplete/dist', // 1
        'node_modules/govuk_frontend_toolkit/stylesheets', // 1
        'node_modules/govuk-elements-sass/public/sass'     // 2
      ]
    }).on('error', sass.logError))
    .pipe(gulp.dest('./static/sass/'));
});
gulp.task('js', () => {
  return gulp.src('./js/*.js')
    .pipe(jsImport({hideConsole: true}))
    .pipe(gulp.dest('./static/js/'));
});
gulp.task('default', ['sass', 'js']);