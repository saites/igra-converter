import EntryLine from './Entry.js'
import { ref, computed } from 'vue'

export default {
  components: {
    EntryLine,
  },
  props: {
    results: Array,
    relevant: Object,
  },
  setup(props) {
    return {}
  },
  template: `
    <div v-for="entry in results">
      <entry-line
          :contestant=entry.registration.contestant
          :events=entry.registration.events
          :found="entry.found ? relevant[entry.found] : null"
          :issues=entry.issues
          :partners=entry.partners
          :relevant=relevant
      ></entry-line>
    </div>
  `
}