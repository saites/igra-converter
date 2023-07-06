import { ref, computed } from 'vue'

export default {
  props: {
    registration: Object,
  },
  setup(props) {

    return {
    }
  },
  template: `
  <div>
      <contestant-line :entry=entry></entry-line>
    {{registration["contestant"]}}
  </div>
  `
}